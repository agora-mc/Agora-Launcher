//! Builds a shareable crash report.
//!
//! Agora diagnoses crashes locally (see `crash_investigator`). When the local
//! signals aren't enough, the user takes the evidence somewhere better — an AI
//! assistant of their choosing, or the community Discord. This module assembles
//! that evidence into one self-contained, redacted markdown document.
//!
//! Everything here is offline string building. Nothing in this module performs
//! network access, and the report is only ever handed back to the user; where
//! it goes next is their decision.

use serde::Serialize;

/// The evidence Agora has gathered about one crash.
#[derive(Debug, Clone, Serialize)]
pub struct CrashReportContext {
    pub instance_id: Option<String>,
    pub crash_log: Option<String>,
    pub crash_signatures: Option<String>,
    pub suspects: Option<String>,
}

/// Preamble telling whoever reads the report what it is and what's wanted.
/// Written for a capable reader — human or model — rather than tuned to any
/// one assistant.
const REPORT_HEADER: &str = "\
# Minecraft crash report

Exported from Agora, a Minecraft mod launcher. File paths and usernames have \
been removed.

Please identify the most likely cause of this crash and what to change to fix \
it. If a specific mod is responsible, say which and explain what in the \
evidence points to it.";

/// Orientation for whoever reads the report.
///
/// A crash log alone gets generic answers, because nothing in it explains what
/// Agora is or what the user can actually do about the crash. This section
/// supplies the vocabulary used above, the authoritative link for anything
/// about Agora itself, and the house rules that keep an answer safe to follow —
/// reversible steps, verified downloads, evidence cited from the log.
///
/// Kept deliberately provider-neutral: it is read by a person on Discord as
/// often as by a model.
const AGORA_BRIEFING: &str = "\
## About Agora (context for whoever is helping)

Agora is a free, open-source, ad-free Minecraft mod launcher.

- Source code, releases and issue tracker: https://github.com/agora-mc/Agora-Launcher
- Community help and curation: https://discord.gg/56tpsa2sTZ

Terms used above:

- **Instance** — one isolated Minecraft installation with its own version, mod \
loader, mods, config and saves. Changing one instance never affects another.
- **Catalog** — Agora's curated, community-reviewed content set, compiled into \
a signed database. Mods may also come from Modrinth when the user enables it. \
Every download is verified by SHA-256.
- **Crash Doctor** — the local analyser that produced this report. It matches \
curated crash signatures and ranks suspect mods by a weighted score over \
stack-frame attribution, past confirmed causes, and co-occurrence across \
crashes. A higher score means a stronger match, not a proven cause.
- **Snapshots** — Agora can restore an instance's previous mod set, so any \
change suggested here is reversible from inside the app.
- **Launch modes** — direct launch runs Minecraft inside Agora; delegated \
launch hands off to the official Mojang launcher and is the default, so Agora \
works without a Microsoft sign-in.

How to give a useful answer:

- Prefer actions the user can take in Agora's interface over hand-editing files.
- Recommend one reversible change at a time, verified by launching the game, so \
cause and effect stay clear. Taking a snapshot first costs nothing.
- Treat the ranked suspects as evidence, not a verdict. Say which line of the \
log supports your conclusion, and say so when you are guessing.
- A mod must match the instance's Minecraft version *and* loader. Check both \
before recommending anything be installed.
- Never advise disabling hash or signature verification, and never point the \
user at unofficial mod mirrors. Suggest the in-app catalog or the mod's \
official page.
- If nothing implicates a mod, consider Java version, GPU drivers, allocated \
memory, shaders, or world corruption.
- For questions about Agora itself — features, settings, governance, curation — \
the repository above and the in-app Help & Guide are authoritative. Please \
don't guess at Agora's behaviour; point the user at those, or at the Discord \
for help from people who use Agora daily.";

/// The closing ask, appended only when there is actual evidence above it.
const REPORT_FOOTER: &str = "\
## What I need

Based on the evidence above: what most likely caused this crash, and what \
should I change? If you are not confident, say what additional information \
would help.";

/// Shown instead of a report when Agora has nothing to export. Keeps the
/// button honest rather than emitting an empty document.
const NO_EVIDENCE: &str = "\
Agora has no crash evidence to export yet. Run the game and let it crash, or \
paste a crash log into Crash Doctor first.";

/// Build the report.
///
/// `manifest_path`, when supplied and readable, adds the instance's mod list —
/// usually the single most useful thing after the log itself, since most crash
/// answers depend on knowing what is installed.
pub fn build_crash_report(
    manifest_path: Option<std::path::PathBuf>,
    context: &CrashReportContext,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Crash data leaves the machine when the user shares this. A Minecraft
    // crash report routinely carries the OS home directory (and therefore the
    // user's real name), and on some loaders the launch command line. Redact
    // before it can be pasted anywhere.
    if let Some(ref crash_log) = context.crash_log {
        parts.push(format!(
            "## Crash Log\n\n```\n{}\n```",
            crate::log_sanitizer::sanitize_log_lines(crash_log)
        ));
    }

    if let Some(ref crash_signatures) = context.crash_signatures {
        parts.push(format!(
            "## Crash Signatures Matched\n\n{}",
            crate::log_sanitizer::sanitize_log_lines(crash_signatures)
        ));
    }

    if let Some(ref suspects) = context.suspects {
        parts.push(format!("## Ranked Suspect Mods\n\n{}", suspects));
    }

    if let Some(ref manifest_path) = manifest_path {
        if let Some(section) = build_mod_list_section(manifest_path) {
            parts.push(section);
        }
    }

    if parts.is_empty() {
        return NO_EVIDENCE.to_string();
    }

    // Evidence first so it is not buried, then the context needed to read it,
    // then the ask.
    format!(
        "{REPORT_HEADER}\n\n{}\n\n{AGORA_BRIEFING}\n\n{REPORT_FOOTER}\n",
        parts.join("\n\n")
    )
}

/// The instance's installed mods, or `None` when the manifest is missing or
/// unreadable — an absent mod list is a weaker report, never a failed one.
fn build_mod_list_section(manifest_path: &std::path::Path) -> Option<String> {
    if !manifest_path.exists() {
        return None;
    }
    let manifest = crate::helpers::read_manifest(manifest_path).ok()?;

    let mod_lines: Vec<String> = manifest
        .mods
        .iter()
        .map(|entry| {
            format!(
                "- {} v{} (source: {})",
                entry.filename,
                entry.version.as_deref().unwrap_or("unknown"),
                entry.source
            )
        })
        .collect();

    if mod_lines.is_empty() {
        return None;
    }

    Some(format!(
        "## Instance: {}\n\n{} mod(s) installed:\n\n{}",
        manifest.name,
        mod_lines.len(),
        mod_lines.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> CrashReportContext {
        CrashReportContext {
            instance_id: Some("inst-1".into()),
            crash_log: None,
            crash_signatures: None,
            suspects: None,
        }
    }

    #[test]
    fn empty_context_reports_that_there_is_nothing_to_export() {
        let report = build_crash_report(None, &context());
        assert_eq!(report, NO_EVIDENCE);
        // No prompt scaffolding around an empty report.
        assert!(!report.contains("## What I need"));
    }

    #[test]
    fn report_carries_header_evidence_and_ask() {
        let ctx = CrashReportContext {
            crash_log: Some("java.lang.NullPointerException".into()),
            suspects: Some("1. examplemod.jar".into()),
            ..context()
        };
        let report = build_crash_report(None, &ctx);

        assert!(report.starts_with("# Minecraft crash report"));
        assert!(report.contains("## Crash Log"));
        assert!(report.contains("java.lang.NullPointerException"));
        assert!(report.contains("## Ranked Suspect Mods"));
        assert!(report.contains("examplemod.jar"));
        assert!(report.trim_end().ends_with("would help."));
        // Sections the context didn't supply are omitted, not left blank.
        assert!(!report.contains("## Crash Signatures Matched"));
    }

    /// The report is read by people and models with no idea what Agora is, so
    /// the orientation section ships with every report, and it must carry the
    /// one link that can answer questions this report cannot.
    #[test]
    fn every_report_carries_the_agora_briefing() {
        let ctx = CrashReportContext {
            crash_log: Some("boom".into()),
            ..context()
        };
        let report = build_crash_report(None, &ctx);

        assert!(report.contains("## About Agora"));
        assert!(report.contains("https://github.com/agora-mc/Agora-Launcher"));
        // Where a person, rather than a model, can help.
        assert!(report.contains("https://discord.gg/56tpsa2sTZ"));
        // Guidance that keeps an answer safe to act on.
        assert!(report.contains("one reversible change at a time"));
        assert!(report.contains("Never advise disabling hash or signature verification"));

        // Evidence comes before the briefing, so the log is never buried under
        // boilerplate for a human skimming the paste.
        let evidence_at = report.find("## Crash Log").unwrap();
        let briefing_at = report.find("## About Agora").unwrap();
        let ask_at = report.find("## What I need").unwrap();
        assert!(
            evidence_at < briefing_at,
            "evidence must precede the briefing"
        );
        assert!(briefing_at < ask_at, "the ask must come last");
    }

    /// No evidence means no report, and therefore no boilerplate either —
    /// pasting a page of Agora background with no crash in it helps nobody.
    #[test]
    fn the_briefing_is_not_emitted_without_evidence() {
        let report = build_crash_report(None, &context());
        assert!(!report.contains("## About Agora"));
    }

    /// The report is built to be pasted elsewhere, so redaction is not
    /// optional — a raw Minecraft log carries the user's home directory.
    #[test]
    fn crash_log_is_redacted_before_it_can_be_shared() {
        let ctx = CrashReportContext {
            crash_log: Some("at C:\\Users\\jarjarpfeil\\AppData\\Roaming\\.minecraft\\mods".into()),
            ..context()
        };
        let report = build_crash_report(None, &ctx);
        assert!(
            !report.contains("jarjarpfeil"),
            "username must not survive into a shareable report: {report}"
        );
    }

    #[test]
    fn a_missing_manifest_weakens_the_report_but_does_not_fail_it() {
        let ctx = CrashReportContext {
            crash_log: Some("boom".into()),
            ..context()
        };
        let report = build_crash_report(Some("does/not/exist.json".into()), &ctx);
        assert!(report.contains("## Crash Log"));
        assert!(!report.contains("mod(s) installed"));
    }
}
