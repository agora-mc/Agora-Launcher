//! `agora bisect`: find the mod that breaks a launch by turning halves of the
//! enabled mods off and on.
//!
//! The scriptable subcommands map one-to-one onto
//! `agora_game_minecraft::bisect_service::BisectService`, which the desktop app uses too.
//! `run` is the interactive loop on top of the same calls: it applies a trial,
//! asks the person to launch the game themselves, and records the answer. This
//! command never launches the game.

use std::io::{BufRead, IsTerminal, Write};

use agora_core::bisect::BisectStatus;
use agora_core::ctx::Ctx;
use agora_game_minecraft::bisect_service::{BisectService, BisectView};
use clap::Subcommand;

use crate::OutputFormat;

#[derive(Subcommand)]
pub enum BisectCmd {
    /// Start a bisect over the enabled mods. Turns nothing off yet.
    Start {
        instance: String,
        #[arg(
            long = "suspect",
            value_name = "FILENAME",
            help = "A mod the crash report implicated; tested first (repeatable)"
        )]
        suspects: Vec<String>,
    },
    /// Show the bisect and its next trial. Changes nothing.
    Status { instance: String },
    /// Turn the mods of the next trial off or on, as the trial says.
    Apply { instance: String },
    /// Record that the last launch worked fine.
    Good { instance: String },
    /// Record that the last launch still showed the problem.
    Bad { instance: String },
    /// Undo the last answer and take the other half next time.
    Back { instance: String },
    /// Stop the bisect and put every mod back the way it was.
    Cancel { instance: String },
    /// Walk through the bisect: apply a trial, you launch the game, you answer.
    Run {
        instance: String,
        #[arg(
            long = "suspect",
            value_name = "FILENAME",
            help = "A mod the crash report implicated; tested first (only when starting)"
        )]
        suspects: Vec<String>,
    },
}

pub async fn run_bisect_command(
    action: BisectCmd,
    ctx: &Ctx,
    output_fmt: OutputFormat,
) -> anyhow::Result<()> {
    let json = output_fmt.is_json_output();
    let service = BisectService::new(ctx.clone());
    match action {
        BisectCmd::Start { instance, suspects } => {
            ensure_instance(ctx, &instance)?;
            let view = service.start(&instance, &suspects)?;
            if json {
                print_json(&view)?;
            } else {
                println!(
                    "Started a bisect over {} enabled mod(s) on '{instance}'. Nothing is turned off yet.",
                    baseline_count(&view)?
                );
                println!("Next: `agora bisect apply {instance}`, launch the game, then `agora bisect good|bad {instance}`.");
                println!("Or `agora bisect run {instance}` walks you through each round.");
            }
        }
        BisectCmd::Status { instance } => {
            ensure_instance(ctx, &instance)?;
            let view = service.view(&instance)?;
            if json {
                print_json(&view)?;
            } else {
                print_status(&instance, &view);
            }
        }
        BisectCmd::Apply { instance } => {
            ensure_instance(ctx, &instance)?;
            let view = service.apply_trial(&instance)?;
            if json {
                print_json(&view)?;
            } else {
                print_applied(&instance, &view);
            }
        }
        BisectCmd::Good { instance } => {
            ensure_instance(ctx, &instance)?;
            let view = service.record_outcome(&instance, false)?;
            print_answer(&instance, &view, json)?;
        }
        BisectCmd::Bad { instance } => {
            ensure_instance(ctx, &instance)?;
            let view = service.record_outcome(&instance, true)?;
            print_answer(&instance, &view, json)?;
        }
        BisectCmd::Back { instance } => {
            ensure_instance(ctx, &instance)?;
            let view = service.step_back(&instance)?;
            if json {
                print_json(&view)?;
            } else {
                println!("Stepped back on '{instance}'. Next trial:");
                print_status(&instance, &view);
            }
        }
        BisectCmd::Cancel { instance } => {
            ensure_instance(ctx, &instance)?;
            let was_running = service.view(&instance)?.session.is_some();
            service.cancel(&instance)?;
            if json {
                let status = if was_running {
                    "cancelled"
                } else {
                    "not_running"
                };
                println!(
                    "{}",
                    serde_json::json!({"status": status, "instanceId": instance})
                );
            } else if was_running {
                println!("Stopped the bisect on '{instance}'. Every mod is back the way it was.");
            } else {
                println!("No bisect was running on '{instance}'. Nothing to restore.");
            }
        }
        BisectCmd::Run { instance, suspects } => {
            if json {
                anyhow::bail!(
                    "`agora bisect run` is interactive and does not print JSON. Use `start`, `apply`, `good`, `bad`, `back` and `cancel` with --json instead."
                );
            }
            ensure_instance(ctx, &instance)?;
            if !std::io::stdin().is_terminal() {
                anyhow::bail!(
                    "`agora bisect run` walks you through launches and needs an interactive terminal. From a script, use `agora bisect start`, `apply`, `good`, `bad`, `back` and `cancel`."
                );
            }
            run_interactive(&service, &instance, &suspects)?;
        }
    }
    Ok(())
}

/// The interactive loop. Applies each trial, asks the person to launch the
/// game and answer, and stops when the bisect reaches a conclusion. Ctrl+C
/// leaves the session and the current trial in place; `agora bisect cancel`
/// restores the mods later.
fn run_interactive(
    service: &BisectService,
    instance: &str,
    suspects: &[String],
) -> anyhow::Result<()> {
    let mut input = std::io::stdin().lock();
    let mut view = service.view(instance)?;
    if view.session.is_none() {
        view = service.start(instance, suspects)?;
        println!(
            "Started a bisect over {} enabled mod(s) on '{instance}'.",
            baseline_count(&view)?
        );
    } else if !suspects.is_empty() {
        anyhow::bail!(
            "A bisect is already running on '{instance}', and --suspect only applies when starting one. Finish it, or run `agora bisect cancel {instance}` first."
        );
    }
    println!(
        "Each round turns some mods off. If you stop early with Ctrl+C, they stay as the current round left them until you run `agora bisect cancel {instance}`."
    );

    loop {
        let awaiting = matches!(
            view.trial.as_ref().map(|t| &t.status),
            Some(BisectStatus::AwaitingTrial)
        );
        if !awaiting {
            print_conclusion(&view);
            println!("Your mods are still as the last round left them.");
            if confirm_restore(&mut input)? {
                service.cancel(instance)?;
                println!("Every mod is back the way it was.");
            } else {
                println!("Left as they are. Run `agora bisect cancel {instance}` to restore them.");
            }
            return Ok(());
        }

        view = service.apply_trial(instance)?;
        print_round(&view);
        match read_answer(&mut input, instance)? {
            Choice::Good => view = service.record_outcome(instance, false)?,
            Choice::Bad => view = service.record_outcome(instance, true)?,
            Choice::Back => match service.step_back(instance) {
                Ok(next) => view = next,
                // Nothing to go back to: say so and ask again on the same round.
                Err(error) => println!("{error}"),
            },
            Choice::Stop => {
                service.cancel(instance)?;
                println!("Stopped. Every mod is back the way it was.");
                return Ok(());
            }
        }
    }
}

#[derive(Debug)]
enum Choice {
    Good,
    Bad,
    Back,
    Stop,
}

/// Read answers until one of the four is typed. End of input is an error, not a
/// stop: the mods stay as they are, so the message says how to put them back.
fn read_answer(input: &mut impl BufRead, instance: &str) -> anyhow::Result<Choice> {
    loop {
        print!("Your answer [w]orks fine / [c]rashes / [b]ack / [x] stop and restore: ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            anyhow::bail!(
                "Input closed before an answer. The bisect is still running and your mods are as the current round left them. Run `agora bisect cancel {instance}` to restore them."
            );
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "w" | "works" => return Ok(Choice::Good),
            "c" | "crashes" => return Ok(Choice::Bad),
            "b" | "back" => return Ok(Choice::Back),
            "x" | "stop" => return Ok(Choice::Stop),
            _ => println!("Please type w, c, b or x."),
        }
    }
}

/// Ask whether to put the mods back once the bisect has a conclusion. Closed
/// input means no: the mods stay as they are and the caller says how to restore.
fn confirm_restore(input: &mut impl BufRead) -> anyhow::Result<bool> {
    print!("Put every mod back the way it was before the bisect? [Y/n]: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(false);
    }
    Ok(!matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "n" | "no"
    ))
}

fn ensure_instance(ctx: &Ctx, instance: &str) -> anyhow::Result<()> {
    let dir = ctx.paths.instance_dir(instance)?;
    if !dir.exists() {
        anyhow::bail!("Instance '{}' not found", instance);
    }
    Ok(())
}

/// Number of mods a bisect started over. A view without a session cannot come
/// back from a successful start, so it is an error rather than a zero.
fn baseline_count(view: &BisectView) -> anyhow::Result<usize> {
    view.session
        .as_ref()
        .map(|s| s.baseline_enabled.len())
        .ok_or_else(|| anyhow::anyhow!("The bisect did not start."))
}

fn print_json(view: &BisectView) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(view)?);
    Ok(())
}

fn print_status(instance: &str, view: &BisectView) {
    let Some(session) = view.session.as_ref() else {
        println!(
            "No bisect is running on '{instance}'. Start one with `agora bisect start {instance}`."
        );
        return;
    };
    println!(
        "Bisect on '{instance}': {} mod(s) still suspected, {} answer(s) so far.",
        session.suspects.len(),
        session
            .history
            .iter()
            .filter(|s| s.outcome.is_some())
            .count()
    );
    match view.trial.as_ref().map(|t| &t.status) {
        Some(BisectStatus::AwaitingTrial) => print_round_details(view),
        _ => print_conclusion(view),
    }
}

fn print_applied(instance: &str, view: &BisectView) {
    if let Some(trial) = view.trial.as_ref() {
        println!(
            "Applied the next trial on '{instance}': {} mod(s) turned off.",
            trial.disable.len()
        );
    }
    println!(
        "Launch the game yourself, then run `agora bisect good {instance}` or `agora bisect bad {instance}`."
    );
}

/// Output for `good` and `bad`: the JSON view, or the next step in words.
fn print_answer(instance: &str, view: &BisectView, json: bool) -> anyhow::Result<()> {
    if json {
        return print_json(view);
    }
    match view.trial.as_ref().map(|t| &t.status) {
        Some(BisectStatus::AwaitingTrial) => {
            println!("Recorded. Next: `agora bisect apply {instance}`.");
        }
        _ => {
            println!("Recorded.");
            print_conclusion(view);
        }
    }
    Ok(())
}

fn print_round(view: &BisectView) {
    println!();
    print_round_details(view);
    println!("Launch Minecraft yourself with the mods as they are now (the official launcher, or `agora launch` in another terminal).");
    println!("  w  it worked fine");
    println!("  c  it still crashes or shows the problem");
    println!("  b  step back and try the other half");
    println!("  x  stop and put every mod back");
}

fn print_round_details(view: &BisectView) {
    let (Some(session), Some(trial)) = (view.session.as_ref(), view.trial.as_ref()) else {
        return;
    };
    println!(
        "Trial {}: about {} more {} to go.",
        trial.completed_trials + 1,
        trial.remaining_trials,
        if trial.remaining_trials == 1 {
            "try"
        } else {
            "tries"
        }
    );
    println!(
        "Turned off this round ({}): {}",
        trial.disable.len(),
        join_or_none(&trial.disable)
    );
    let left_on: Vec<String> = session
        .baseline_enabled
        .iter()
        .filter(|f| !trial.disable.contains(*f))
        .cloned()
        .collect();
    println!("Left on ({}): {}", left_on.len(), join_or_none(&left_on));
}

fn print_conclusion(view: &BisectView) {
    let Some(trial) = view.trial.as_ref() else {
        return;
    };
    match &trial.status {
        BisectStatus::AwaitingTrial => {}
        BisectStatus::Culprit { filename } => {
            println!(
                "Found it: {filename} is the one. Removing or updating it should fix the problem."
            );
        }
        BisectStatus::CulpritGroup { filenames } => {
            println!(
                "Narrowed to these, which have to be turned off together: {}",
                join_or_none(filenames)
            );
        }
        BisectStatus::Inconclusive => {
            println!(
                "Every mod was cleared, so this is not one mod on its own. It may be a combination, a config file, or something outside the mod list."
            );
        }
    }
}

fn join_or_none(names: &[String]) -> String {
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn an_answer_is_read_until_one_of_the_four_choices_is_typed() {
        // Blank and unknown lines are asked again; case and spaces do not matter.
        let mut input = Cursor::new("\nmaybe\n  W \n");
        assert!(matches!(
            read_answer(&mut input, "demo").unwrap(),
            Choice::Good
        ));
        let mut input = Cursor::new("C\n");
        assert!(matches!(
            read_answer(&mut input, "demo").unwrap(),
            Choice::Bad
        ));
        let mut input = Cursor::new("back\n");
        assert!(matches!(
            read_answer(&mut input, "demo").unwrap(),
            Choice::Back
        ));
        let mut input = Cursor::new("x\n");
        assert!(matches!(
            read_answer(&mut input, "demo").unwrap(),
            Choice::Stop
        ));
    }

    #[test]
    fn closed_input_before_an_answer_is_an_error_that_names_the_way_out() {
        let mut input = Cursor::new("");
        let error = read_answer(&mut input, "demo").unwrap_err().to_string();
        assert!(error.contains("Input closed"), "{error}");
        assert!(error.contains("agora bisect cancel demo"), "{error}");
    }

    #[test]
    fn restore_is_the_default_and_closed_input_leaves_mods_alone() {
        assert!(confirm_restore(&mut Cursor::new("\n")).unwrap());
        assert!(confirm_restore(&mut Cursor::new("yes\n")).unwrap());
        assert!(!confirm_restore(&mut Cursor::new("n\n")).unwrap());
        assert!(!confirm_restore(&mut Cursor::new("NO\n")).unwrap());
        assert!(!confirm_restore(&mut Cursor::new("")).unwrap());
    }

    #[test]
    fn a_start_with_no_session_is_an_error_not_a_zero_count() {
        let empty = BisectView {
            session: None,
            trial: None,
        };
        assert!(baseline_count(&empty).is_err());
    }
}
