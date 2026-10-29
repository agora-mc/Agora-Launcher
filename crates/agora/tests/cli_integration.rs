//! Integration tests for the `agora` CLI binary.
//!
//! These tests execute the compiled `agora` binary against a temporary
//! `--data-dir` to verify paths, settings, instance list/show, JSON output
//! purity, error envelopes, exit codes, and import registration.  Tests
//! avoid network access and never touch the user's real data root.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Path to the compiled `agora` binary, set by Cargo for integration tests.
const AGORA_BIN: &str = env!("CARGO_BIN_EXE_agora");
const TEST_MSA_CREDENTIALS_ENV: &str = "AGORA_TEST_MSA_CREDENTIALS_JSON";
/// Per-command ceiling.  Commands normally finish in well under a second, so
/// this is a backstop against a genuine hang, not a performance assertion.
/// It is deliberately generous: on a loaded Windows CI runner an I/O stall has
/// taken a plain `agora paths` past 30s while the rest of the suite passed.
const CLI_TIMEOUT: Duration = Duration::from_secs(120);

fn agora_command(data_dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(AGORA_BIN);
    cmd.arg("--data-dir").arg(data_dir);
    cmd.args(args);
    cmd.env_remove(TEST_MSA_CREDENTIALS_ENV);
    cmd
}

fn run_command(mut cmd: Command, args: &[&str]) -> std::process::Output {
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to execute agora binary");

    // Drain both pipes on their own threads.  The stdio pipe buffer is only a
    // few kilobytes, and a command that fills it blocks in `write` until
    // someone reads, which the poll below would report as a command timeout
    // rather than the harness deadlock it actually is.
    let mut stdout_pipe = child.stdout.take().expect("stdout was piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr was piped");
    let stdout_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });

    let deadline = Instant::now() + CLI_TIMEOUT;
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status, false),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let status = child.wait().expect("failed to reap timed-out agora");
                break (status, true);
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("failed while waiting for agora command {args:?}: {error}");
            }
        }
    };

    // Killing the child closes its write ends, so both readers finish here.
    let stdout = stdout_reader.join().expect("stdout reader panicked");
    let stderr = stderr_reader.join().expect("stderr reader panicked");

    if timed_out {
        panic!(
            "agora command timed out after {:?}: {:?}\nstdout:\n{}\nstderr:\n{}",
            CLI_TIMEOUT,
            args,
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr),
        );
    }

    std::process::Output {
        status,
        stdout,
        stderr,
    }
}

/// Helper: run `agora --data-dir <dir>` with the given args.
fn run_agora(data_dir: &Path, args: &[&str]) -> std::process::Output {
    run_command(agora_command(data_dir, args), args)
}

/// Helper: run with `--json` flag prepended.
fn run_agora_json(data_dir: &Path, args: &[&str]) -> std::process::Output {
    let mut full_args: Vec<&str> = vec!["--json"];
    full_args.extend_from_slice(args);
    run_agora(data_dir, &full_args)
}

fn test_msa_credentials_json() -> String {
    serde_json::to_string(&MsaCredentials {
        username: "test".into(),
        uuid: "00000000-0000-0000-0000-000000000000".into(),
        access_token: "test_access_token".into(),
        refresh_token: "test_refresh_token".into(),
        expires: chrono::Utc::now() + chrono::Duration::hours(1),
        client_id: agora_game_minecraft::msa::AGORA_MSA_CLIENT_ID.into(),
        auth_version: agora_game_minecraft::msa::MSA_AUTH_VERSION,
    })
    .expect("serialize fake MSA credentials")
}

fn run_agora_with_test_credentials(data_dir: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = agora_command(data_dir, args);
    cmd.env(TEST_MSA_CREDENTIALS_ENV, test_msa_credentials_json());
    run_command(cmd, args)
}

fn run_agora_json_with_test_credentials(data_dir: &Path, args: &[&str]) -> std::process::Output {
    let mut full_args: Vec<&str> = vec!["--json"];
    full_args.extend_from_slice(args);
    run_agora_with_test_credentials(data_dir, &full_args)
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn tempdir() -> TempDir {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "agora-cli-integration-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).expect("failed to create temp dir");
    TempDir { path }
}

/// Create a temp dir for use as `--data-dir`.
fn temp_data_dir() -> (TempDir, PathBuf) {
    let tmp = tempdir();
    let path = tmp.path().to_path_buf();
    (tmp, path)
}

/// Assert stdout is valid JSON and return the parsed value.
fn assert_json_stdout(output: &std::process::Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout must be valid JSON: {e}\nstdout:\n{stdout}"))
}

const TOP_LEVEL_COMMANDS: &[&str] = &[
    "list-instances",
    "paths",
    "get-instance",
    "instance",
    "mod",
    "health",
    "inventory",
    "registry",
    "snapshots",
    "import",
    "launch",
    "auth",
    "sync",
    "runtime",
    "loader",
    "settings",
    "skill",
    "mcp",
    "crash",
    "migrate-data",
    "pack",
    "export",
    "loadout",
    "lockfile",
    "plugin",
    "provider",
    "games",
    "template",
    "bisect",
    "governance",
    "technic",
    "backup",
    "world",
    "server",
    "log",
    "proxy",
    "skin",
];

const NESTED_COMMANDS: &[&[&str]] = &[
    &["instance", "create"],
    &["instance", "clone"],
    &["instance", "rename"],
    &["instance", "lock"],
    &["instance", "unlock"],
    &["instance", "repair-loader"],
    &["instance", "recommend-memory"],
    &["instance", "delete"],
    &["instance", "deleted"],
    &["instance", "restore"],
    &["instance", "purge"],
    &["instance", "health"],
    &["instance", "revert"],
    &["instance", "loader-version"],
    &["instance", "java"],
    &["instance", "wrapper"],
    &["instance", "window"],
    &["mod", "graph"],
    &["mod", "orphans"],
    &["mod", "why"],
    &["mod", "disable-plan"],
    &["template", "list"],
    &["template", "create"],
    &["template", "delete"],
    &["template", "apply"],
    &["mod", "list"],
    &["mod", "search"],
    &["mod", "install"],
    &["mod", "remove"],
    &["mod", "update"],
    &["mod", "update-all"],
    &["mod", "enable"],
    &["mod", "disable"],
    &["registry", "status"],
    &["registry", "sync"],
    &["snapshots", "list"],
    &["snapshots", "create"],
    &["snapshots", "restore"],
    &["snapshots", "delete"],
    &["auth", "login"],
    &["auth", "status"],
    &["auth", "logout"],
    &["auth", "list"],
    &["auth", "switch"],
    &["world", "list"],
    &["world", "rename"],
    &["world", "copy"],
    &["world", "delete"],
    &["world", "export"],
    &["world", "import"],
    &["server", "list"],
    &["server", "add"],
    &["server", "remove"],
    &["log", "list"],
    &["log", "share"],
    &["proxy", "show"],
    &["proxy", "set"],
    &["skin", "show"],
    &["skin", "wear"],
    &["skin", "cape"],
    &["instance", "hooks"],
    &["instance", "notes"],
    &["instance", "group"],
    &["instance", "share"],
    &["runtime", "list"],
    &["runtime", "ensure"],
    &["runtime", "remove-unused"],
    &["runtime", "inspect"],
    &["loader", "list"],
    &["loader", "install"],
    &["settings", "list"],
    &["settings", "get"],
    &["settings", "set"],
    &["mcp", "serve"],
    &["crash", "list"],
    &["crash", "inspect"],
    &["crash", "investigate"],
    &["pack", "install"],
    &["pack", "export"],
    &["pack", "review"],
    &["pack", "import"],
    &["import", "launchers"],
    &["import", "plan"],
    &["import", "run"],
    &["backup", "export"],
    &["backup", "import"],
    &["backup", "prune"],
    &["loadout", "create"],
    &["loadout", "list"],
    &["loadout", "apply"],
    &["loadout", "update"],
    &["loadout", "delete"],
    &["bisect", "start"],
    &["bisect", "status"],
    &["bisect", "apply"],
    &["bisect", "good"],
    &["bisect", "bad"],
    &["bisect", "back"],
    &["bisect", "cancel"],
    &["bisect", "run"],
    &["lockfile", "export"],
    &["lockfile", "verify"],
    &["lockfile", "repair"],
    &["lockfile", "import"],
    &["instance", "updates"],
    &["mod", "pin"],
    &["mod", "unpin"],
    &["mod", "changelog"],
    &["pack", "update-preview"],
    &["pack", "update"],
    &["plugin", "list"],
    &["plugin", "preview"],
    &["plugin", "install"],
    &["plugin", "enable"],
    &["plugin", "disable"],
    &["plugin", "remove"],
    &["plugin", "log"],
    &["plugin", "disable-all"],
    &["plugin", "check-update"],
    &["plugin", "update"],
    &["plugin", "restore-data"],
    &["plugin", "keygen"],
    &["plugin", "sign"],
    &["games", "discover"],
    &["games", "list"],
    &["games", "folder"],
    &["games", "folder", "add"],
    &["games", "folder", "remove"],
    &["games", "folder", "list"],
    &["games", "base"],
    &["games", "base", "build"],
    &["games", "base", "list"],
    &["games", "base", "verify"],
    &["games", "base", "remove"],
    &["games", "catalog"],
    &["games", "catalog", "list"],
    &["games", "catalog", "install"],
    &["games", "content"],
    &["games", "content", "add"],
    &["games", "content", "list"],
    &["games", "content", "show"],
    &["games", "content", "verify"],
    &["games", "content", "remove"],
    &["games", "content", "fomod"],
    &["games", "content", "fomod", "show"],
    &["games", "content", "fomod", "install"],
    &["games", "instance"],
    &["games", "instance", "create"],
    &["games", "instance", "list"],
    &["games", "instance", "launch"],
    &["games", "instance", "check"],
    &["games", "instance", "delete"],
    &["games", "instance", "content"],
    &["games", "instance", "content", "add"],
    &["games", "instance", "content", "list"],
    &["games", "instance", "content", "remove"],
    &["games", "instance", "content", "enable"],
    &["games", "instance", "content", "disable"],
    &["games", "instance", "content", "move"],
    &["games", "instance", "deploy"],
    &["games", "instance", "undeploy"],
    &["games", "instance", "set-deployment"],
    &["games", "instance", "plugins"],
    &["games", "instance", "plugins", "enable"],
    &["games", "instance", "plugins", "disable"],
    &["games", "instance", "plugins", "sort"],
    &["games", "instance", "plugins", "move"],
    &["games", "instance", "plugins", "lock"],
    &["games", "instance", "plugins", "unlock"],
    &["games", "instance", "plugins", "check"],
    &["games", "instance", "tools"],
    &["games", "instance", "tools", "list"],
    &["games", "instance", "tools", "run"],
    &["games", "instance", "tools", "rollback"],
    &["games", "instance", "tools", "remove"],
    &["games", "instance", "tools", "diff"],
    &["games", "instance", "ini"],
    &["games", "instance", "saves"],
    &["games", "launch"],
    &["games", "user-files"],
    &["games", "user-files", "status"],
    &["games", "user-files", "restore"],
    &["games", "tools-swap"],
    &["games", "tools-swap", "status"],
    &["games", "tools-swap", "restore"],
    &["games", "tools-swap", "remove-copy"],
    &["games", "import"],
    &["games", "import", "mo2"],
    &["games", "import", "mo2", "scan"],
    &["plugin", "settings"],
    &["plugin", "set"],
    &["governance", "summary"],
    &["governance", "events"],
    &["governance", "config"],
    &["governance", "diagnostics"],
    &["registry", "under-review"],
    &["registry", "reviews"],
    &["registry", "audit-log"],
    &["registry", "resolutions"],
    &["technic", "search"],
    &["technic", "install"],
];

// ---------------------------------------------------------------------------
// Public help contract
// ---------------------------------------------------------------------------

#[test]
fn version_flag_prints_package_version() {
    let mut command = Command::new(AGORA_BIN);
    command.arg("--version");
    let output = run_command(command, &["--version"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("agora {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn top_level_help_lists_public_commands_and_global_flags() {
    let mut command = Command::new(AGORA_BIN);
    command.arg("--help");
    let output = run_command(command, &["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for command in TOP_LEVEL_COMMANDS {
        assert!(
            stdout.contains(command),
            "missing command {command} in:\n{stdout}"
        );
    }
    for flag in [
        "--data-dir",
        "--json",
        "--output",
        "--registry-repo",
        "--log-file",
    ] {
        assert!(
            stdout.contains(flag),
            "missing global flag {flag} in:\n{stdout}"
        );
    }
}

#[test]
fn skill_prints_the_agent_guide_without_touching_the_data_root() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["skill"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("---\nname: agora\n"), "{stdout}");
    // An agent reads the guide before it is pointed at a data root, so
    // printing it must not create one.
    assert!(!data_dir.join("local_state.db").exists());
}

#[test]
fn every_command_help_exits_zero() {
    let (_tmp, data_dir) = temp_data_dir();
    for command in TOP_LEVEL_COMMANDS {
        let output = run_agora(&data_dir, &[*command, "--help"]);
        assert!(
            output.status.success(),
            "{command} --help failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for command in NESTED_COMMANDS {
        let mut args = command.to_vec();
        args.push("--help");
        let output = run_agora(&data_dir, &args);
        assert!(
            output.status.success(),
            "{} --help failed:\n{}",
            command.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn critical_command_flags_remain_in_help() {
    let (_tmp, data_dir) = temp_data_dir();
    for (command, flags) in [
        (&["launch"][..], &["--yes", "--timings"][..]),
        (&["import"][..], &["--url", "--symlink-saves"][..]),
        (
            &["mod", "install"][..],
            &[
                "--dry-run",
                "--include-optional",
                "--exclude-optional",
                "--replace-conflicts",
                "--abort-conflicts",
                "--allow-replace",
                "--skip-health-scan",
            ][..],
        ),
        (
            &["mod", "remove"][..],
            &["--remove-anyway", "--abort-conflicts"][..],
        ),
        (&["mod", "enable"][..], &["--with-deps"][..]),
        (&["mcp", "serve"][..], &["--stdio"][..]),
    ] {
        let mut args = command.to_vec();
        args.push("--help");
        let output = run_agora(&data_dir, &args);
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        for flag in flags {
            assert!(
                stdout.contains(flag),
                "missing {flag} from {} help:\n{stdout}",
                command.join(" ")
            );
        }
    }
}

#[test]
fn documentation_examples_use_public_command_names() {
    let docs = include_str!("../../../docs/CLI.md");
    for flag in [
        "--data-dir",
        "--output",
        "--yes",
        "--timings",
        "--url",
        "--dry-run",
        "--include-optional",
        "--exclude-optional",
        "--replace-conflicts",
        "--abort-conflicts",
        "--remove-anyway",
        "--allow-replace",
        "--skip-health-scan",
        "--stdio",
    ] {
        assert!(
            docs.contains(flag),
            "docs/CLI.md is missing critical flag {flag}"
        );
    }
    let mut checked = 0;
    let mut in_fence = false;
    for line in docs.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            continue;
        }
        let Some(example) = line.trim().strip_prefix("agora ") else {
            continue;
        };
        if example.starts_with('[') {
            continue;
        }
        let tokens: Vec<_> = example.split_whitespace().collect();
        if tokens
            .iter()
            .any(|token| matches!(*token, "--version" | "--help"))
        {
            checked += 1;
            continue;
        }
        let command = tokens
            .iter()
            .find(|token| TOP_LEVEL_COMMANDS.contains(token))
            .unwrap_or_else(|| panic!("documented example has no public command: {line}"));
        assert!(TOP_LEVEL_COMMANDS.contains(command));
        checked += 1;
    }
    assert!(
        checked >= 40,
        "expected broad CLI example coverage, found {checked}"
    );
}

#[test]
fn ndjson_is_rejected_until_a_streaming_contract_exists() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["--output", "ndjson", "paths"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid value 'ndjson'"));
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

#[test]
fn paths_human_shows_all_expected_keys() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["paths"]);
    assert!(output.status.success(), "agora paths should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("root: {}", data_dir.display())),
        "stdout should contain root: {}\n{}",
        data_dir.display(),
        stdout,
    );
    assert!(stdout.contains("local_state_db:"));
    assert!(stdout.contains("instances:"));
    assert!(stdout.contains("minecraft_runtime:"));
    assert!(stdout.contains("snapshots:"));
    assert!(stdout.contains("locks:"));
    assert!(stdout.contains("loader_cache:"));
    assert!(stdout.contains("java_runtimes:"));
    assert!(stdout.contains("staging:"));

    // Human mode must NOT emit JSON objects to stdout
    assert!(
        !stdout.contains(r#""root""#),
        "human mode should not emit JSON:\n{stdout}"
    );
}

#[test]
fn paths_json_has_expected_keys() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora_json(&data_dir, &["paths"]);
    assert!(output.status.success(), "agora paths --json should succeed");

    let parsed = assert_json_stdout(&output);
    let obj = parsed
        .as_object()
        .expect("paths JSON output should be an object");

    assert!(obj.contains_key("root"), "missing 'root' key");
    assert!(
        obj.contains_key("local_state_db"),
        "missing 'local_state_db'"
    );
    assert!(obj.contains_key("instances"), "missing 'instances'");
    assert!(
        obj.contains_key("minecraft_runtime"),
        "missing 'minecraft_runtime'"
    );
    assert!(obj.contains_key("loader_cache"), "missing 'loader_cache'");
    assert!(obj.contains_key("java_runtimes"), "missing 'java_runtimes'");
    assert!(obj.contains_key("snapshots"), "missing 'snapshots'");
    assert!(obj.contains_key("staging"), "missing 'staging'");
    assert!(obj.contains_key("locks"), "missing 'locks'");

    assert_eq!(
        obj["root"].as_str().unwrap(),
        data_dir.to_string_lossy(),
        "root path should match data dir",
    );
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[test]
fn settings_set_and_get_string() {
    let (_tmp, data_dir) = temp_data_dir();

    // Set — pass a JSON string literal so it is stored as a JSON string
    let set = run_agora(&data_dir, &["settings", "set", "greeting", r#""hello""#]);
    assert!(
        set.status.success(),
        "settings set failed:\n{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let set_stdout = String::from_utf8_lossy(&set.stdout);
    assert!(
        set_stdout.contains("hello"),
        "stdout should contain value:\n{set_stdout}"
    );

    // Get
    let get = run_agora(&data_dir, &["settings", "get", "greeting"]);
    assert!(
        get.status.success(),
        "settings get failed:\n{}",
        String::from_utf8_lossy(&get.stderr)
    );
    let get_stdout = String::from_utf8_lossy(&get.stdout);
    assert!(
        get_stdout.contains("hello"),
        "get stdout should contain value:\n{get_stdout}"
    );
}

#[test]
fn settings_set_and_get_json() {
    let (_tmp, data_dir) = temp_data_dir();

    // Set in JSON mode
    let set = run_agora_json(&data_dir, &["settings", "set", "theme", r#""dark""#]);
    assert!(set.status.success(), "settings set --json failed");

    let set_parsed = assert_json_stdout(&set);
    assert_eq!(set_parsed["status"], "set");
    assert_eq!(set_parsed["key"], "theme");
    assert_eq!(set_parsed["value"], "dark");

    // Get in JSON mode
    let get = run_agora_json(&data_dir, &["settings", "get", "theme"]);
    let get_parsed = assert_json_stdout(&get);
    assert_eq!(get_parsed, "dark");
}

#[test]
fn settings_set_boolean() {
    let (_tmp, data_dir) = temp_data_dir();

    let set = run_agora(&data_dir, &["settings", "set", "flag", "true"]);
    assert!(set.status.success(), "settings set boolean failed");

    let get = run_agora_json(&data_dir, &["settings", "get", "flag"]);
    let get_parsed = assert_json_stdout(&get);
    assert_eq!(get_parsed, serde_json::Value::Bool(true));
}

#[test]
fn settings_set_json_object_and_get() {
    let (_tmp, data_dir) = temp_data_dir();

    let set = run_agora_json(&data_dir, &["settings", "set", "obj", r#"{"nested":42}"#]);
    assert!(set.status.success(), "settings set object failed");

    let set_parsed = assert_json_stdout(&set);
    assert_eq!(set_parsed["value"]["nested"], 42);

    let get = run_agora_json(&data_dir, &["settings", "get", "obj"]);
    let get_parsed = assert_json_stdout(&get);
    assert_eq!(get_parsed["nested"], 42);
}

#[test]
fn settings_get_unknown_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["settings", "get", "nonexistent"]);
    assert!(
        !output.status.success(),
        "expected non-zero exit for unknown key"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "stderr should mention 'not found':\n{stderr}"
    );
}

#[test]
fn settings_list_persists_value() {
    let (_tmp, data_dir) = temp_data_dir();

    // Set a value, then list
    run_agora(&data_dir, &["settings", "set", "my_key", r#""my_val""#]);
    let list = run_agora_json(&data_dir, &["settings", "list"]);
    let parsed = assert_json_stdout(&list);
    let obj = parsed
        .as_object()
        .expect("settings list JSON should be an object");
    assert_eq!(obj.get("my_key").and_then(|v| v.as_str()), Some("my_val"));
}

// ---------------------------------------------------------------------------
// Instance list and show
// ---------------------------------------------------------------------------

#[test]
fn instance_list_human_succeeds() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["list-instances"]);
    assert!(output.status.success(), "list-instances should succeed");
    // Context initialization may emit warnings to stderr; that is normal.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("No local state database"),
        "should not report missing DB (context initializes it)",
    );
}

#[test]
fn instance_list_json_empty_array() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora_json(&data_dir, &["list-instances"]);
    assert!(
        output.status.success(),
        "list-instances --json should succeed"
    );

    let parsed = assert_json_stdout(&output);
    assert_eq!(
        parsed,
        serde_json::json!([]),
        "JSON output should be an empty array"
    );
}

#[test]
fn instance_get_unknown_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["get-instance", "nonexistent"]);
    assert!(
        !output.status.success(),
        "get-instance should fail for unknown ID"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("database"),
        "stderr should mention 'not found':\n{stderr}",
    );
}

// ---------------------------------------------------------------------------
// JSON output purity and error envelopes
// ---------------------------------------------------------------------------

#[test]
fn json_stdout_is_parseable() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora_json(&data_dir, &["paths"]);
    assert!(output.status.success());
    assert_json_stdout(&output); // panics if not valid JSON
}

#[test]
fn json_error_envelope_on_failure() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora_json(&data_dir, &["get-instance", "missing-instance"]);
    assert!(!output.status.success(), "expected non-zero exit");

    // The error envelope is emitted to stderr, but stderr may also contain
    // context-warning lines before the JSON payload.  Find the last JSON
    // object by extracting from the first '{' to the matching '}'.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let json_start = stderr.rfind('{').unwrap_or_else(|| {
        panic!("stderr must contain a JSON envelope:\n{stderr}");
    });
    let json_end = stderr[json_start..]
        .rfind('}')
        .map(|i| json_start + i + 1)
        .unwrap_or_else(|| {
            panic!("stderr JSON envelope must have a closing '}}':\n{stderr}");
        });
    let json_text = &stderr[json_start..json_end];

    let parsed: serde_json::Value = serde_json::from_str(json_text)
        .unwrap_or_else(|e| panic!("failed to parse JSON envelope: {e}\ntext:\n{json_text}"));

    assert!(
        parsed.get("error").is_some(),
        "error envelope must contain 'error' field",
    );
    assert!(
        parsed.get("exitCode").is_some(),
        "error envelope must contain 'exitCode' field",
    );
    let code = parsed["exitCode"].as_i64().unwrap();
    assert!(code > 0, "exitCode must be > 0 on error, got {code}");
}

#[test]
fn json_stdout_empty_stderr_on_success() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora_json(&data_dir, &["paths"]);
    assert!(output.status.success());
    // Warnings from CoreContext::initialize go to stderr, but that is normal.
    // The important thing is stdout is valid JSON.
    assert_json_stdout(&output);
}

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

#[test]
fn exit_code_for_nonexistent_import_path() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["import", "Z:\\does-not-exist\\instance"]);
    assert!(
        !output.status.success(),
        "import of nonexistent path should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not exist"),
        "stderr should mention 'does not exist':\n{stderr}"
    );
}

#[test]
fn exit_code_for_unsupported_file_extension() {
    let (_tmp, data_dir) = temp_data_dir();

    // Create a dummy file with unsupported extension
    let src = tempdir();
    let bad_file = src.path().join("instance.xyz");
    std::fs::write(&bad_file, b"garbage").unwrap();

    let output = run_agora(&data_dir, &["import", &bad_file.to_string_lossy()]);
    assert!(
        !output.status.success(),
        "import of unsupported file should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Unsupported"),
        "stderr should mention 'Unsupported':\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Import registration (local directory, no network)
// ---------------------------------------------------------------------------

#[test]
fn import_directory_creates_instance() {
    let (_tmp, data_dir) = temp_data_dir();

    // Create a source directory that looks like a simple Minecraft instance
    let src = tempdir();
    let instance_src = src.path().join("my-test-instance");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();
    std::fs::write(
        instance_src.join("mods").join("useful-mod.jar"),
        b"fake jar",
    )
    .unwrap();

    let output = run_agora(&data_dir, &["import", &instance_src.to_string_lossy()]);
    assert!(
        output.status.success(),
        "import directory should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Imported"),
        "stdout should say 'Imported':\n{stdout}"
    );

    // List instances — should show the imported instance
    let list = run_agora_json(&data_dir, &["list-instances"]);
    let list_parsed = assert_json_stdout(&list);
    let instances: Vec<serde_json::Value> = serde_json::from_value(list_parsed).unwrap();
    assert_eq!(instances.len(), 1, "should have 1 instance");
    assert_eq!(instances[0]["name"], "my-test-instance");
    assert_eq!(instances[0]["instance_id"], "my-test-instance");

    // Get the instance detail
    let show = run_agora_json(&data_dir, &["get-instance", "my-test-instance"]);
    let show_parsed = assert_json_stdout(&show);
    assert_eq!(show_parsed["name"], "my-test-instance");
    assert_eq!(show_parsed["instance_id"], "my-test-instance");
    assert_eq!(show_parsed["is_modpack"], serde_json::Value::Bool(false));
    assert_eq!(show_parsed["is_locked"], serde_json::Value::Bool(false));
}

#[test]
fn import_directory_json_output() {
    let (_tmp, data_dir) = temp_data_dir();

    let src = tempdir();
    let instance_src = src.path().join("json-import");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();
    std::fs::write(instance_src.join("mods").join("a.jar"), b"mod content").unwrap();

    let output = run_agora_json(&data_dir, &["import", &instance_src.to_string_lossy()]);
    assert!(output.status.success(), "import --json should succeed");

    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["name"], "json-import");
    assert_eq!(parsed["instance_id"], "json-import");
    assert!(
        parsed.get("imported_mods").is_some(),
        "JSON output should include imported_mods"
    );
}

#[test]
fn import_twice_reuses_name_with_suffix() {
    let (_tmp, data_dir) = temp_data_dir();

    let src1 = tempdir();
    let inst1 = src1.path().join("duplicate-name");
    std::fs::create_dir_all(inst1.join("mods")).unwrap();
    std::fs::write(inst1.join("mods").join("a.jar"), b"mod").unwrap();
    let out1 = run_agora(&data_dir, &["import", &inst1.to_string_lossy()]);
    assert!(
        out1.status.success(),
        "first import should succeed (status: {}):\nstdout:\n{}\nstderr:\n{}",
        out1.status,
        String::from_utf8_lossy(&out1.stdout),
        String::from_utf8_lossy(&out1.stderr),
    );

    let src2 = tempdir();
    let inst2 = src2.path().join("duplicate-name");
    std::fs::create_dir_all(inst2.join("mods")).unwrap();
    std::fs::write(inst2.join("mods").join("b.jar"), b"other").unwrap();
    let out2 = run_agora(&data_dir, &["import", &inst2.to_string_lossy()]);
    assert!(
        !out2.status.success(),
        "second import with duplicate name should fail"
    );
    let stderr = String::from_utf8_lossy(&out2.stderr);
    assert!(
        stderr.contains("already exists"),
        "should mention 'already exists':\n{stderr}",
    );
}

// ---------------------------------------------------------------------------
// Instance delete
// ---------------------------------------------------------------------------

#[test]
fn instance_delete_nonexistent_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["instance", "delete", "no-such-instance"]);
    assert!(
        !output.status.success(),
        "instance delete on nonexistent instance should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "stderr should mention 'not found':\n{stderr}"
    );
}

#[test]
fn instance_delete_after_import_succeeds() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let instance_src = src.path().join("to-delete");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();
    std::fs::write(instance_src.join("mods").join("a.jar"), b"mod").unwrap();

    let import = run_agora(&data_dir, &["import", &instance_src.to_string_lossy()]);
    assert!(import.status.success(), "import should succeed");

    let delete = run_agora(&data_dir, &["instance", "delete", "to-delete"]);
    assert!(delete.status.success(), "delete should succeed");
    let stdout = String::from_utf8_lossy(&delete.stdout);
    assert!(
        stdout.contains("Deleted"),
        "human output should contain 'Deleted':\n{stdout}"
    );

    // Verify it's gone
    let get = run_agora(&data_dir, &["get-instance", "to-delete"]);
    assert!(!get.status.success(), "instance should no longer exist");
}

#[test]
fn instance_delete_json_output() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let instance_src = src.path().join("json-delete");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();
    std::fs::write(instance_src.join("mods").join("a.jar"), b"mod").unwrap();

    run_agora(&data_dir, &["import", &instance_src.to_string_lossy()]);

    let output = run_agora_json(&data_dir, &["instance", "delete", "json-delete"]);
    assert!(output.status.success(), "delete --json should succeed");
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["status"], "deleted");
    assert_eq!(parsed["instanceId"], "json-delete");
}

// ---------------------------------------------------------------------------
// Recently deleted: list, restore, purge
// ---------------------------------------------------------------------------

/// Import a small instance, delete it, and return its Recently deleted entry ID.
fn import_and_delete(data_dir: &Path, name: &str) -> String {
    let src = tempdir();
    let instance_src = src.path().join(name);
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();
    std::fs::write(instance_src.join("mods").join("a.jar"), b"mod").unwrap();
    let import = run_agora(data_dir, &["import", &instance_src.to_string_lossy()]);
    assert!(import.status.success(), "import should succeed");
    let delete = run_agora(data_dir, &["instance", "delete", name]);
    assert!(delete.status.success(), "delete should succeed");
    let listed = assert_json_stdout(&run_agora_json(data_dir, &["instance", "deleted"]));
    listed
        .as_array()
        .expect("instance deleted --json is an array")
        .iter()
        .find(|entry| entry["instance_id"] == name)
        .unwrap_or_else(|| panic!("{name} is not in Recently deleted: {listed}"))["entry_id"]
        .as_str()
        .expect("entry_id is a string")
        .to_string()
}

#[test]
fn instance_deleted_lists_nothing_on_a_fresh_data_root() {
    let (_tmp, data_dir) = temp_data_dir();
    let json = run_agora_json(&data_dir, &["instance", "deleted"]);
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    assert_eq!(assert_json_stdout(&json), serde_json::json!([]));
    let human = run_agora(&data_dir, &["instance", "deleted"]);
    assert!(human.status.success());
    assert!(String::from_utf8_lossy(&human.stdout).contains("empty"));
}

#[test]
fn deleted_instance_is_listed_with_id_name_time_and_size() {
    let (_tmp, data_dir) = temp_data_dir();
    let entry_id = import_and_delete(&data_dir, "listed-pack");

    let parsed = assert_json_stdout(&run_agora_json(&data_dir, &["instance", "deleted"]));
    let entry = &parsed[0];
    assert_eq!(entry["entry_id"], entry_id.as_str());
    assert_eq!(entry["name"], "listed-pack");
    assert!(entry["deleted_at"].is_string(), "{entry}");
    assert!(entry["size_bytes"].as_u64().unwrap_or(0) >= 3, "{entry}");

    let human = run_agora(&data_dir, &["instance", "deleted"]);
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(stdout.contains(&entry_id), "{stdout}");
    assert!(stdout.contains("listed-pack"), "{stdout}");
    let row = stdout
        .lines()
        .find(|line| line.contains("listed-pack"))
        .unwrap_or_else(|| panic!("no row for listed-pack:\n{stdout}"));
    assert!(
        row.trim_end().ends_with(" B") || row.trim_end().ends_with(" KiB"),
        "a small instance should show its size in B or KiB: {row}"
    );
}

#[test]
fn restore_brings_a_deleted_instance_back_and_empties_the_list() {
    let (_tmp, data_dir) = temp_data_dir();
    let entry_id = import_and_delete(&data_dir, "restore-me");
    assert!(!run_agora(&data_dir, &["get-instance", "restore-me"])
        .status
        .success());

    let restore = run_agora_json(&data_dir, &["instance", "restore", &entry_id]);
    assert!(
        restore.status.success(),
        "{}",
        String::from_utf8_lossy(&restore.stderr)
    );
    let parsed = assert_json_stdout(&restore);
    assert_eq!(parsed["status"], "restored");
    assert_eq!(parsed["entryId"], entry_id.as_str());
    assert_eq!(parsed["instanceId"], "restore-me");
    assert_eq!(parsed["renamed"], false);

    assert!(run_agora(&data_dir, &["get-instance", "restore-me"])
        .status
        .success());
    let paths = assert_json_stdout(&run_agora_json(&data_dir, &["paths"]));
    let mod_file = Path::new(paths["instances"].as_str().unwrap())
        .join("restore-me")
        .join("mods")
        .join("a.jar");
    assert!(
        mod_file.is_file(),
        "the mod file came back with the instance"
    );
    assert_eq!(
        assert_json_stdout(&run_agora_json(&data_dir, &["instance", "deleted"])),
        serde_json::json!([])
    );
}

#[test]
fn restore_refuses_unknown_and_traversal_entry_ids() {
    let (_tmp, data_dir) = temp_data_dir();
    for entry in ["no-such-entry", "../../outside", "..\\..\\outside"] {
        let output = run_agora(&data_dir, &["instance", "restore", entry]);
        assert!(!output.status.success(), "restore {entry} should fail");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("no longer available"), "{stderr}");
    }
}

#[test]
fn purge_without_yes_refuses_without_a_terminal_and_keeps_the_entry() {
    let (_tmp, data_dir) = temp_data_dir();
    let entry_id = import_and_delete(&data_dir, "keep-me");

    // The test harness gives the child no terminal, so the prompt cannot run.
    let output = run_agora(&data_dir, &["instance", "purge", &entry_id]);
    assert!(!output.status.success(), "purge must not run unconfirmed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--yes"), "{stderr}");

    let listed = assert_json_stdout(&run_agora_json(&data_dir, &["instance", "deleted"]));
    assert_eq!(listed[0]["entry_id"], entry_id.as_str());
}

#[test]
fn purge_unknown_entry_fails_even_with_yes() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["instance", "purge", "no-such-entry", "--yes"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No deleted instance"), "{stderr}");
}

// ---------------------------------------------------------------------------
// Instance rename
// ---------------------------------------------------------------------------

#[test]
fn instance_rename_after_import_succeeds() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let instance_src = src.path().join("old-name");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();

    run_agora(&data_dir, &["import", &instance_src.to_string_lossy()]);

    let rename = run_agora(&data_dir, &["instance", "rename", "old-name", "New Name"]);
    assert!(rename.status.success(), "rename should succeed");
    let stdout = String::from_utf8_lossy(&rename.stdout);
    assert!(
        stdout.contains("New Name") && stdout.contains("old-name"),
        "stdout should mention both old and new name:\n{stdout}"
    );

    // Verify under new name
    let get = run_agora_json(&data_dir, &["get-instance", "old-name"]);
    let parsed = assert_json_stdout(&get);
    assert_eq!(parsed["name"], "New Name");
}

#[test]
fn instance_rename_json_output() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let instance_src = src.path().join("json-rename");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();

    run_agora(&data_dir, &["import", &instance_src.to_string_lossy()]);

    let output = run_agora_json(
        &data_dir,
        &["instance", "rename", "json-rename", "Json Renamed"],
    );
    assert!(output.status.success(), "rename --json should succeed");
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["status"], "renamed");
    assert_eq!(parsed["instanceId"], "json-rename");
    assert_eq!(parsed["name"], "Json Renamed");
}

// ---------------------------------------------------------------------------
// Running beside the desktop app
// ---------------------------------------------------------------------------

#[test]
fn another_process_sees_what_the_cli_changed() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let instance_src = src.path().join("shared");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();
    std::fs::write(instance_src.join("mods").join("a.jar"), b"mod").unwrap();
    run_agora(&data_dir, &["import", &instance_src.to_string_lossy()]);

    // This test process stands in for the desktop app watching the same data.
    let locks_root = agora_core::app_paths::AppPaths::from_root(data_dir.clone()).locks_root();
    let mut feed = agora_core::change_feed::ChangeFeed::new(&locks_root);

    let disable = run_agora(&data_dir, &["mod", "disable", "shared", "a.jar"]);
    assert!(
        disable.status.success(),
        "{}",
        String::from_utf8_lossy(&disable.stderr)
    );
    let changes = feed.poll();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].instance_id, "shared");
    assert_eq!(changes[0].program, "Agora CLI");
    assert_eq!(changes[0].operation, "disable_mod");

    // A database-only edit takes no instance lock but is still reported.
    let rename = run_agora(&data_dir, &["instance", "rename", "shared", "Renamed"]);
    assert!(rename.status.success());
    let changes = feed.poll();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].operation, "rename");

    assert!(feed.poll().is_empty());
}

// ---------------------------------------------------------------------------
// Snapshot (no instance — should fail gracefully)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_list_nonexistent_instance_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["snapshots", "list", "no-such-instance"]);
    assert!(
        !output.status.success(),
        "snapshot list on nonexistent instance should fail"
    );
}

#[test]
fn snapshot_delete_nonexistent_instance_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(
        &data_dir,
        &["snapshots", "delete", "no-such-instance", "snap-1"],
    );
    assert!(
        !output.status.success(),
        "snapshot delete on nonexistent instance should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "stderr should mention 'not found':\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Health on nonexistent instance
// ---------------------------------------------------------------------------

#[test]
fn health_nonexistent_instance_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["health", "no-such-instance"]);
    assert!(
        !output.status.success(),
        "health on nonexistent instance should fail"
    );
}

#[test]
fn crash_investigate_uses_latest_log_without_crash_report() {
    let (_tmp, data_dir) = temp_data_dir();
    run_agora(&data_dir, &["paths"]);
    create_vanilla_instance(&data_dir, "crash-latest", "1.21");
    let logs = data_dir.join("instances/crash-latest/logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
        logs.join("latest.log"),
        "[Render thread/ERROR]: java.lang.OutOfMemoryError: Java heap space\n",
    )
    .unwrap();

    let output = run_agora_json(&data_dir, &["crash", "investigate", "crash-latest"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["failure_category"], "Oom");
    assert_eq!(
        parsed["evidence"]["sources"][0]["meta"]["basename"],
        "latest.log"
    );
}

#[test]
fn crash_investigate_accepts_explicit_text_evidence() {
    let (_tmp, data_dir) = temp_data_dir();
    run_agora(&data_dir, &["paths"]);
    create_vanilla_instance(&data_dir, "crash-explicit", "1.21");
    let extra = data_dir.join("selected-forge-crash.txt");
    std::fs::write(
        &extra,
        "net.minecraftforge.fml.ModLoadingException: Missing mandatory dependency\nCaused by: java.lang.NoClassDefFoundError: example/api/EntryPoint\n",
    )
    .unwrap();

    let output = run_agora_json(
        &data_dir,
        &[
            "crash",
            "investigate",
            "crash-explicit",
            "--file",
            extra.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(
        parsed["evidence"]["sources"][0]["meta"]["basename"],
        "selected-forge-crash.txt"
    );
    assert!(parsed["evidence"]["sources"][0].get("path").is_none());
}

#[test]
fn instance_memory_recommendation_is_read_only_and_explained() {
    let (_tmp, data_dir) = temp_data_dir();
    run_agora(&data_dir, &["paths"]);
    create_vanilla_instance(&data_dir, "memory-recommend", "1.21");
    let conn = agora_core::db::local_state_connection(&data_dir.join("local_state.db")).unwrap();
    let before = agora_core::db::get_instance(&conn, "memory-recommend")
        .unwrap()
        .unwrap();

    let output = run_agora_json(
        &data_dir,
        &["instance", "recommend-memory", "memory-recommend"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let recommendation = assert_json_stdout(&output);
    assert_eq!(recommendation["recommended_mb"], 2048);
    assert!(recommendation["explanation"]
        .as_str()
        .unwrap_or_default()
        .contains("enabled mods"));

    let after = agora_core::db::get_instance(&conn, "memory-recommend")
        .unwrap()
        .unwrap();
    assert_eq!(before.jvm_memory_mb, after.jvm_memory_mb);
    assert_eq!(before.jvm_memory_mode, after.jvm_memory_mode);
}

// ---------------------------------------------------------------------------
// Registry status (no registry.db — should not panic)
// ---------------------------------------------------------------------------

#[test]
fn registry_status_does_not_panic() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["registry", "status"]);
    // This should either succeed or fail gracefully — but never panic
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panic"),
        "registry status should not panic:\n{stderr}"
    );
}

#[test]
fn registry_status_no_desktop_button_instruction() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["registry", "status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains("Click") && !stderr.contains("Click"),
        "registry status must not tell terminal users to click a desktop button:\n{stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("agora registry sync"),
        "registry status should point terminal users at the CLI command:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Launch helpers
// ---------------------------------------------------------------------------

use agora_core::download::sha1_hex;
use agora_game_minecraft::msa::MsaCredentials;

/// Platform key used in natives directory name.
fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

/// Pre-populate the Minecraft runtime cache so `materialize()` runs without
/// network.  All `sha1`/`size` fields in the synthetic version JSON match
/// the dummy files we create below, producing cache hits in every
/// `download_sha1_atomic` call.
///
/// `java_major` must match the system Java version that will be discovered.
fn populate_runtime_cache(minecraft_runtime: &Path, version: &str, java_major: i64) {
    let versions_dir = minecraft_runtime.join("versions").join(version);
    let libraries_dir = minecraft_runtime.join("libraries");
    let assets_dir = minecraft_runtime.join("assets");
    let natives_dir = minecraft_runtime
        .join("natives")
        .join(version)
        .join(platform());

    std::fs::create_dir_all(&versions_dir).unwrap();
    std::fs::create_dir_all(&libraries_dir).unwrap();
    std::fs::create_dir_all(assets_dir.join("indexes")).unwrap();
    std::fs::create_dir_all(&natives_dir).unwrap();

    // --- Client JAR ---------------------------------------------------------
    let client_bytes = b"agora-test-client-jar";
    std::fs::write(versions_dir.join(format!("{version}.jar")), client_bytes).unwrap();

    // --- Library (fake Mojang-hosted artifact) ------------------------------
    let lib_rel = "net/minecraft/minecraft/1.21/minecraft-1.21.jar";
    let lib_path = libraries_dir.join(lib_rel);
    std::fs::create_dir_all(lib_path.parent().unwrap()).unwrap();
    let lib_bytes = b"agora-test-library";
    std::fs::write(&lib_path, lib_bytes).unwrap();

    // --- Asset index (empty objects map) ------------------------------------
    let asset_json = br#"{"objects":{}}"#;
    std::fs::write(
        assets_dir.join("indexes").join(format!("{version}.json")),
        asset_json,
    )
    .unwrap();

    // --- Version JSON with matching sha1/size -------------------------------
    let version_json = serde_json::json!({
        "id": version,
        "mainClass": "net.minecraft.client.main.Main",
        "type": "release",
        "libraries": [{
            "name": "net.minecraft:minecraft:1.21",
            "downloads": {
                "artifact": {
                    "path": lib_rel,
                    "url": "https://piston-data.mojang.com/v1/objects/0000000000000000000000000000000000000000/fake.jar",
                    "sha1": sha1_hex(lib_bytes),
                    "size": lib_bytes.len()
                }
            }
        }],
        "assetIndex": {
            "id": version,
            "url": "https://piston-meta.mojang.com/v1/packages/0000000000000000000000000000000000000000/fake.json",
            "sha1": sha1_hex(asset_json),
            "size": asset_json.len() as i64
        },
        "downloads": {
            "client": {
                "url": "https://piston-data.mojang.com/v1/objects/0000000000000000000000000000000000000000/client.jar",
                "sha1": sha1_hex(client_bytes),
                "size": client_bytes.len() as i64
            }
        },
        "javaVersion": {
            "component": "java-runtime-gamma",
            "majorVersion": java_major
        }
    });

    std::fs::write(
        versions_dir.join(format!("{version}.json")),
        serde_json::to_vec(&version_json).unwrap(),
    )
    .unwrap();
}

/// Write a minimal `InstanceManifest` and upsert an `InstanceRow` into the
/// local state DB so the CLI launch command finds a valid instance.
fn create_vanilla_instance(data_dir: &Path, instance_id: &str, version: &str) {
    let instance_dir = data_dir.join("instances").join(instance_id);
    std::fs::create_dir_all(instance_dir.join("mods")).unwrap();

    let manifest = serde_json::json!({
        "instance_id": instance_id,
        "name": instance_id,
        "minecraft_version": version,
        "loader": "vanilla",
        "loader_version": "",
        "is_locked": false,
        "mods": [],
        "resourcepacks": [],
        "shaders": [],
        "datapacks": [],
        "worlds": [],
        "user_preferences": {}
    });
    std::fs::write(
        instance_dir.join("instance_manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    let db_path = data_dir.join("local_state.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    agora_core::db::upsert_instance(
        &conn,
        &agora_core::models::InstanceRow {
            instance_id: instance_id.to_string(),
            name: instance_id.to_string(),
            minecraft_version: version.to_string(),
            loader: "vanilla".into(),
            loader_version: String::new(),
            is_modpack: false,
            is_locked: false,
            last_launched_at: None,
            jvm_memory_mb: 4096,
            jvm_memory_mode: "manual".into(),
            jvm_gc: "g1gc".into(),
            jvm_custom_args: String::new(),
            jvm_always_pre_touch: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            java_path: None,
            java_incompatible_override: false,
            icon_path: None,
            launch_mode_override: "auto".into(),
            import_source: None,
            total_play_ms: 0,
        },
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Launch helpers
// ---------------------------------------------------------------------------

/// Write a fake Java script file that:
///   - Responds to `-XshowSettings:properties -version` (the `inspect_java`
///     probe) by printing a valid-looking Java version line and exiting 0.
///   - For any other invocation (the actual Minecraft launch command), exits
///     with the given `exit_code`.
///
/// `java_major` is the version number reported by the probe (e.g. 8, 21).
fn write_fake_java(dir: &Path, exit_code: i32, java_major: i64) -> PathBuf {
    if cfg!(target_os = "windows") {
        let path = dir.join("fake_java.bat");
        let script = format!(
            "@echo off\r\n\
             if \"%1\"==\"-XshowSettings:properties\" (\r\n\
               if \"%2\"==\"-version\" (\r\n\
                 echo java.specification.version = {java_major}\r\n\
                 echo java.version = {java_major}.0.1\r\n\
                 echo os.arch = amd64\r\n\
                 exit /b 0\r\n\
               )\r\n\
             )\r\n\
             exit /b {exit_code}\r\n"
        );
        std::fs::write(&path, script).unwrap();
        path
    } else {
        let path = dir.join("fake_java.sh");
        let script = format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"-XshowSettings:properties\" ] && [ \"$2\" = \"-version\" ]; then\n\
               echo \"java.specification.version = {java_major}\"\n\
               echo \"java.version = {java_major}.0.1\"\n\
               echo \"os.arch = amd64\"\n\
               exit 0\n\
             fi\n\
             exit {exit_code}\n"
        );
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }
}

/// Set up a full launch environment with fake Java, a pre-populated runtime
/// cache, and a vanilla instance ready for `agora launch`.
fn prepare_launch_state(data_dir: &Path, exit_code: i32, java_major: i64) -> String {
    let version = "1.21";
    let instance_id = "launch-test";
    let minecraft_runtime = data_dir.join("minecraft-runtime");

    populate_runtime_cache(&minecraft_runtime, version, java_major);
    create_vanilla_instance(data_dir, instance_id, version);

    // Pre-create a snapshot and write LKG state so the child process's
    // `create_or_reuse_snapshot` finds a matching index and skips the call.
    let instance_dir = data_dir.join("instances").join(instance_id);
    let snapshot = agora_core::snapshot::create_snapshot(&instance_dir, Some("Initial"))
        .expect("create initial snapshot");
    let lkg = serde_json::json!({
        "currentLkgSnapshotId": snapshot.id,
        "lastPromotedAt": null,
        "lastLaunchSessionId": null,
        "lastLaunchOutcome": null,
        "promotedSnapshotIds": [],
        "schemaVersion": 1
    });
    std::fs::write(
        instance_dir.join("lkg.json"),
        serde_json::to_vec(&lkg).unwrap(),
    )
    .unwrap();

    let fake_path = write_fake_java(data_dir, exit_code, java_major);
    let java_str = fake_path.to_string_lossy();
    let java_val = serde_json::json!(java_str.as_ref()).to_string();
    let set = run_agora_json(data_dir, &["settings", "set", "java_path", &java_val]);
    if !set.status.success() {
        panic!(
            "set java_path failed:\n{}",
            String::from_utf8_lossy(&set.stderr)
        );
    }

    instance_id.to_owned()
}

// ---------------------------------------------------------------------------
// Launch tests
// ---------------------------------------------------------------------------

#[test]
fn launch_invalid_instance_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    // Prime the context so the local state DB is initialised.
    run_agora(&data_dir, &["paths"]);

    let output = run_agora(&data_dir, &["launch", "nonexistent"]);
    assert!(
        !output.status.success(),
        "launch must fail for unknown instance"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "stderr must mention 'not found':\n{stderr}"
    );
}

#[test]
fn launch_fake_java_success() {
    let (_tmp, data_dir) = temp_data_dir();
    run_agora(&data_dir, &["paths"]);

    let instance_id = prepare_launch_state(&data_dir, 0, 21);
    let output = run_agora_with_test_credentials(&data_dir, &["launch", &instance_id, "--yes"]);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "launch with fake Java (exit 0) failed.\nstdout:\n{stdout}\nstderr:\n{stderr}\nexit code: {:?}",
            output.status.code()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Abandoned") || stdout.contains("abandoned"),
        "stdout should mention Abandoned (exit 0, short runtime):\n{stdout}"
    );
}

#[test]
fn launch_migrated_manifest_uses_cold_and_warm_plan_cache() {
    let (_tmp, data_dir) = temp_data_dir();
    run_agora(&data_dir, &["paths"]);
    let instance_id = prepare_launch_state(&data_dir, 0, 21);
    let manifest_path = data_dir
        .join("instances")
        .join(&instance_id)
        .join("instance_manifest.json");
    let manifest = agora_core::helpers::read_manifest(&manifest_path).unwrap();
    agora_core::helpers::atomic_write_manifest(&manifest_path, &manifest).unwrap();

    for cache_state in ["miss", "hit"] {
        let output =
            run_agora_json_with_test_credentials(&data_dir, &["launch", &instance_id, "--yes"]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "{cache_state} launch failed:\n{stderr}"
        );
        assert_json_stdout(&output);
        assert!(
            stderr.contains(&format!("durable plan cache {cache_state}")),
            "{stderr}"
        );
    }
}

#[test]
fn launch_fake_java_crash() {
    let (_tmp, data_dir) = temp_data_dir();
    run_agora(&data_dir, &["paths"]);

    let instance_id = prepare_launch_state(&data_dir, 1, 21);
    let output = run_agora_with_test_credentials(&data_dir, &["launch", &instance_id, "--yes"]);
    assert!(
        !output.status.success(),
        "launch must exit with non-zero code for crash outcome"
    );
    assert_eq!(output.status.code(), Some(7), "crash must exit code 7");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Crash") || stdout.contains("crash"),
        "stdout should mention Crash (non-zero exit):\n{stdout}"
    );
}

#[test]
fn launch_json_stdout() {
    let (_tmp, data_dir) = temp_data_dir();
    run_agora(&data_dir, &["paths"]);

    let instance_id = prepare_launch_state(&data_dir, 0, 21);
    let output =
        run_agora_json_with_test_credentials(&data_dir, &["launch", &instance_id, "--yes"]);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "launch --json failed:\nstderr:\n{stderr}\nexit code: {:?}",
            output.status.code()
        );
    }
    let parsed = assert_json_stdout(&output);
    assert!(parsed.get("pid").is_some(), "JSON must contain 'pid'");
    assert!(
        parsed.get("session_id").is_some(),
        "JSON must contain 'session_id'"
    );
    assert!(
        parsed.get("outcome").is_some(),
        "JSON must contain 'outcome'"
    );
}

#[test]
fn launch_timings_are_written_to_stderr() {
    let (_tmp, data_dir) = temp_data_dir();
    run_agora(&data_dir, &["paths"]);
    let instance_id = prepare_launch_state(&data_dir, 0, 21);
    let output =
        run_agora_with_test_credentials(&data_dir, &["launch", &instance_id, "--yes", "--timings"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[timing]"),
        "expected timing output:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// MCP serve --stdio (JSON-RPC 2.0 over stdio)
// ---------------------------------------------------------------------------

#[test]
fn mcp_serve_stdio_jsonrpc_requests() {
    let (_tmp, data_dir) = temp_data_dir();

    let mut cmd = Command::new(AGORA_BIN);
    cmd.arg("--data-dir").arg(&data_dir);
    cmd.arg("mcp").arg("serve").arg("--stdio");
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .expect("failed to spawn agora mcp serve --stdio");

    let mut child_stdin = child.stdin.take().expect("missing stdin");
    let mut child_stdout = child.stdout.take().expect("missing stdout");
    let mut child_stderr = child.stderr.take().expect("missing stderr");

    // Newline-delimited JSON-RPC 2.0 requests: two with id, one notification (no id)
    let input = r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","method":"notifications/initialized"}
"#;

    child_stdin
        .write_all(input.as_bytes())
        .expect("write to child stdin");
    drop(child_stdin); // close stdin → signals EOF to the child

    // Read all output (responses are small; pipe buffer is large enough)
    let mut stdout_buf = String::new();
    child_stdout
        .read_to_string(&mut stdout_buf)
        .expect("read child stdout");

    let mut stderr_buf = String::new();
    child_stderr
        .read_to_string(&mut stderr_buf)
        .expect("read child stderr");

    let status = child.wait().expect("wait for child");
    assert!(
        status.success(),
        "mcp serve must exit cleanly on EOF, stderr:\n{stderr_buf}"
    );

    // Exactly two response lines (notification produces no response)
    let lines: Vec<&str> = stdout_buf.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "expected 2 JSON-RPC response lines, got {}:\n{:?}",
        lines.len(),
        lines,
    );

    // Line 1 — initialize
    let r1: serde_json::Value = serde_json::from_str(lines[0])
        .unwrap_or_else(|e| panic!("invalid JSON on response 1: {e}\n{}", lines[0]));
    assert_eq!(r1["jsonrpc"], "2.0");
    assert_eq!(r1["id"], 1);
    assert!(r1.get("result").is_some(), "initialize must have 'result'");
    assert!(
        r1.get("error").is_none(),
        "initialize must not have 'error'"
    );
    assert_eq!(r1["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(r1["result"]["serverInfo"]["name"], "agora");

    // Line 2 — tools/list
    let r2: serde_json::Value = serde_json::from_str(lines[1])
        .unwrap_or_else(|e| panic!("invalid JSON on response 2: {e}\n{}", lines[1]));
    assert_eq!(r2["jsonrpc"], "2.0");
    assert_eq!(r2["id"], 2);
    assert!(r2.get("result").is_some(), "tools/list must have 'result'");
    assert!(
        r2.get("error").is_none(),
        "tools/list must not have 'error'"
    );
    let tools = r2["result"]["tools"]
        .as_array()
        .expect("tools/list result.tools must be an array");
    assert!(
        !tools.is_empty(),
        "tools/list should return at least one tool"
    );

    // Stderr must NOT carry JSON-RPC response framing (responses are stdout-only)
    assert!(
        !stderr_buf.contains(r#""jsonrpc":"#),
        "stderr must not contain JSON-RPC responses:\n{stderr_buf}",
    );
}

// ---------------------------------------------------------------------------
// Mod install dry-run — optional/conflict policy, no filesystem mutation
// ---------------------------------------------------------------------------

#[test]
fn mod_install_dry_run_nonexistent_instance_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(
        &data_dir,
        &["mod", "install", "sodium", "no-such-inst", "--dry-run"],
    );
    assert!(
        !output.status.success(),
        "mod install dry-run on nonexistent instance should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "stderr should mention 'not found':\n{stderr}"
    );
}

#[test]
fn mod_install_dry_run_json_envelope() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora_json(
        &data_dir,
        &["mod", "install", "sodium", "no-such-inst", "--dry-run"],
    );
    assert!(
        !output.status.success(),
        "mod install dry-run should fail in JSON mode"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let json_start = stderr.rfind('{').unwrap_or_else(|| {
        panic!("stderr must contain JSON envelope:\n{stderr}");
    });
    let json_end = stderr[json_start..]
        .rfind('}')
        .map(|i| json_start + i + 1)
        .unwrap_or_else(|| {
            panic!("JSON envelope must have closing '}}':\n{stderr}");
        });
    let json_text = &stderr[json_start..json_end];
    let parsed: serde_json::Value = serde_json::from_str(json_text)
        .unwrap_or_else(|e| panic!("failed to parse JSON envelope: {e}\ntext:\n{json_text}"));
    assert!(
        parsed.get("error").is_some(),
        "error envelope must contain 'error'"
    );
    assert!(
        parsed.get("exitCode").is_some(),
        "error envelope must contain 'exitCode'"
    );
}

#[test]
fn mod_install_dry_run_optional_flags_parsed() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(
        &data_dir,
        &[
            "mod",
            "install",
            "sodium",
            "no-such-inst",
            "--dry-run",
            "--exclude-optional",
        ],
    );
    assert!(
        !output.status.success(),
        "mod install with --exclude-optional should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "stderr should mention 'not found':\n{stderr}"
    );
}

#[test]
fn mod_install_dry_run_no_filesystem_mutation() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let instance_src = src.path().join("dry-run-inst");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();
    std::fs::write(instance_src.join("mods").join("a.jar"), b"mod").unwrap();
    let import = run_agora(&data_dir, &["import", &instance_src.to_string_lossy()]);
    assert!(import.status.success(), "import should succeed");

    let pre_files: Vec<_> = std::fs::read_dir(data_dir.join("instances/dry-run-inst/mods"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();

    let output = run_agora(
        &data_dir,
        &[
            "mod",
            "install",
            "nonexistent-mod",
            "dry-run-inst",
            "--dry-run",
        ],
    );
    assert!(
        !output.status.success(),
        "dry-run on unresolved mod should fail"
    );

    let post_files: Vec<_> = std::fs::read_dir(data_dir.join("instances/dry-run-inst/mods"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();

    assert_eq!(
        pre_files.len(),
        post_files.len(),
        "dry-run must not modify filesystem",
    );
}

// ---------------------------------------------------------------------------
// mod enable / disable — missing file must fail loudly
// ---------------------------------------------------------------------------

#[test]
fn mod_disable_missing_file_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "mod-disable-missing", &["real.jar"]);

    let output = run_agora(
        &data_dir,
        &["mod", "disable", "mod-disable-missing", "ghost.jar"],
    );
    assert!(
        !output.status.success(),
        "mod disable on a missing file must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ghost.jar") && stderr.contains("not found"),
        "stderr should name the missing file:\n{stderr}"
    );

    assert!(
        data_dir
            .join("instances/mod-disable-missing/mods")
            .join("real.jar")
            .exists(),
        "existing mods must be untouched"
    );
}

#[test]
fn mod_enable_missing_file_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "mod-enable-missing", &["real.jar"]);

    let output = run_agora(
        &data_dir,
        &["mod", "enable", "mod-enable-missing", "ghost.jar"],
    );
    assert!(
        !output.status.success(),
        "mod enable on a missing file must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ghost.jar") && stderr.contains("not found"),
        "stderr should name the missing file:\n{stderr}"
    );
}

#[test]
fn mod_enable_disable_missing_file_json_exit_code() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "mod-enable-json", &["real.jar"]);

    let output = run_agora_json(
        &data_dir,
        &["mod", "disable", "mod-enable-json", "ghost.jar"],
    );
    assert!(
        !output.status.success(),
        "mod disable --json on a missing file must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let json_start = stderr.rfind('{').unwrap_or_else(|| {
        panic!("stderr must contain JSON envelope:\n{stderr}");
    });
    let json_end = stderr[json_start..]
        .rfind('}')
        .map(|i| json_start + i + 1)
        .unwrap_or_else(|| {
            panic!("JSON envelope must have closing '}}':\n{stderr}");
        });
    let json_text = &stderr[json_start..json_end];
    let parsed: serde_json::Value = serde_json::from_str(json_text)
        .unwrap_or_else(|e| panic!("failed to parse JSON envelope: {e}\ntext:\n{json_text}"));
    assert!(
        parsed["error"].as_str().unwrap_or("").contains("ghost.jar"),
        "{parsed}"
    );
    assert!(parsed["exitCode"].as_i64().unwrap_or(0) != 0, "{parsed}");
}

// ---------------------------------------------------------------------------
// Pack install smoke (local manifest, no network)
// ---------------------------------------------------------------------------

#[test]
fn pack_install_nonexistent_manifest_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(
        &data_dir,
        &["pack", "install", "Z:\\no-such-pack.json", "my-inst"],
    );
    assert!(
        !output.status.success(),
        "pack install with nonexistent manifest should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cannot read"),
        "stderr should mention 'Cannot read':\n{stderr}"
    );
}

#[test]
fn pack_install_nonexistent_instance_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let manifest_path = src.path().join("pack.json");
    std::fs::write(
        &manifest_path,
        r#"{"name":"TestPack","minecraft_version":"1.21","loader":"fabric","loader_version":"0.16.0","mods":[{"id":"sodium","source":"modrinth","status":"required"}]}"#,
    )
    .unwrap();
    let output = run_agora(
        &data_dir,
        &[
            "pack",
            "install",
            &manifest_path.to_string_lossy(),
            "no-such-instance",
        ],
    );
    assert!(
        !output.status.success(),
        "pack install into nonexistent instance should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "stderr should mention 'not found':\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Export — creates expected output
// ---------------------------------------------------------------------------

#[test]
fn export_nonexistent_instance_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let dest = tempdir();
    let output = run_agora(
        &data_dir,
        &[
            "export",
            "no-such-instance",
            &dest.path().join("out").to_string_lossy(),
        ],
    );
    assert!(
        !output.status.success(),
        "export from nonexistent instance should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "stderr should mention 'not found':\n{stderr}"
    );
}

#[test]
fn export_creates_server_mods_directory() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let instance_src = src.path().join("export-test");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();
    std::fs::write(instance_src.join("mods").join("server-mod.jar"), b"server").unwrap();
    std::fs::write(instance_src.join("mods").join("client-mod.jar"), b"client").unwrap();

    let import = run_agora(&data_dir, &["import", &instance_src.to_string_lossy()]);
    assert!(import.status.success(), "import should succeed");

    let dest = tempdir();
    let export_dest = dest.path().join("server");
    let output = run_agora(
        &data_dir,
        &["export", "export-test", &export_dest.to_string_lossy()],
    );
    assert!(
        output.status.success(),
        "export should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Exported"),
        "stdout should contain 'Exported':\n{stdout}"
    );

    assert!(
        export_dest.join("mods").is_dir(),
        "export destination should have mods/ directory"
    );

    let entries: Vec<_> = std::fs::read_dir(export_dest.join("mods"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    assert!(
        entries
            .iter()
            .any(|n| n.to_string_lossy().contains("server")),
        "server-mod should be exported"
    );
}

#[test]
fn export_json_output_valid() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let instance_src = src.path().join("json-export");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();
    std::fs::write(instance_src.join("mods").join("mod.jar"), b"mod").unwrap();

    run_agora(&data_dir, &["import", &instance_src.to_string_lossy()]);

    let dest = tempdir();
    let export_dest = dest.path().join("server-json");
    let output = run_agora_json(
        &data_dir,
        &["export", "json-export", &export_dest.to_string_lossy()],
    );
    assert!(output.status.success(), "export --json should succeed");

    let parsed = assert_json_stdout(&output);
    assert!(
        parsed.get("total_mods").is_some(),
        "JSON must contain 'total_mods'"
    );
    assert!(
        parsed.get("server_mods").is_some(),
        "JSON must contain 'server_mods'"
    );
    assert!(
        parsed.get("removed_client_only").is_some(),
        "JSON must contain 'removed_client_only'"
    );
}

// ---------------------------------------------------------------------------
// Loadout — create / list / apply / delete
// ---------------------------------------------------------------------------

fn create_minimal_instance(data_dir: &std::path::Path, id: &str) {
    // Loadout tests only need a registered instance with some mod files. Set
    // that fixture up directly instead of going through `import`, whose
    // asynchronous recovery snapshot makes these unrelated tests sensitive
    // to Windows filesystem timing under CI load.
    create_loader_instance(data_dir, id, &["sodium.jar", "lithium.jar", "phosphor.jar"]);
}

#[test]
fn loadout_create_succeeds() {
    let (_tmp, data_dir) = temp_data_dir();
    create_minimal_instance(&data_dir, "loadout-test");

    let output = run_agora(
        &data_dir,
        &["loadout", "create", "loadout-test", "my-profile"],
    );
    assert!(
        output.status.success(),
        "loadout create should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Created loadout"),
        "stdout should contain 'Created loadout':\n{stdout}"
    );
}

#[test]
fn loadout_create_json_output() {
    let (_tmp, data_dir) = temp_data_dir();
    create_minimal_instance(&data_dir, "loadout-json");

    let output = run_agora_json(
        &data_dir,
        &["loadout", "create", "loadout-json", "json-prof"],
    );
    assert!(
        output.status.success(),
        "loadout create --json should succeed"
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["name"], "json-prof");
    assert!(parsed.get("enabled_mods").is_some());
    assert!(parsed.get("created_at").is_some());
}

#[test]
fn loadout_list_after_create() {
    let (_tmp, data_dir) = temp_data_dir();
    create_minimal_instance(&data_dir, "loadout-list");

    run_agora(&data_dir, &["loadout", "create", "loadout-list", "alpha"]);
    run_agora(&data_dir, &["loadout", "create", "loadout-list", "beta"]);

    let output = run_agora_json(&data_dir, &["loadout", "list", "loadout-list"]);
    assert!(output.status.success(), "loadout list should succeed");
    let parsed = assert_json_stdout(&output);
    let profiles: Vec<serde_json::Value> = serde_json::from_value(parsed).unwrap();
    assert_eq!(profiles.len(), 2, "should have 2 profiles");
    assert_eq!(profiles[0]["name"], "alpha");
    assert_eq!(profiles[1]["name"], "beta");
}

#[test]
fn loadout_apply_succeeds() {
    let (_tmp, data_dir) = temp_data_dir();
    create_minimal_instance(&data_dir, "loadout-apply");

    run_agora(&data_dir, &["loadout", "create", "loadout-apply", "full"]);
    let output = run_agora(&data_dir, &["loadout", "apply", "loadout-apply", "full"]);
    assert!(
        output.status.success(),
        "loadout apply should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Applied"),
        "stdout should contain 'Applied':\n{stdout}"
    );
}

#[test]
fn loadout_apply_json_output() {
    let (_tmp, data_dir) = temp_data_dir();
    create_minimal_instance(&data_dir, "loadout-apply-json");

    run_agora(
        &data_dir,
        &["loadout", "create", "loadout-apply-json", "prof"],
    );
    let output = run_agora_json(
        &data_dir,
        &["loadout", "apply", "loadout-apply-json", "prof"],
    );
    assert!(
        output.status.success(),
        "loadout apply --json should succeed"
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["status"], "applied");
    assert_eq!(parsed["profile"], "prof");
}

#[test]
fn loadout_apply_saves_an_undo_point_the_snapshot_list_shows() {
    let (_tmp, data_dir) = temp_data_dir();
    create_minimal_instance(&data_dir, "loadout-undo");
    run_agora(&data_dir, &["loadout", "create", "loadout-undo", "full"]);

    let output = run_agora(&data_dir, &["loadout", "apply", "loadout-undo", "full"]);
    assert!(
        output.status.success(),
        "loadout apply should succeed:
{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("before-loadout"),
        "stdout should name the undo point:
{stdout}"
    );

    let list = run_agora_json(&data_dir, &["snapshots", "list", "loadout-undo"]);
    let parsed = assert_json_stdout(&list);
    let snapshots: Vec<serde_json::Value> = serde_json::from_value(parsed).unwrap();
    assert!(
        snapshots.iter().any(|s| s["label"] == "before-loadout"),
        "apply from the CLI must save an undo point: {snapshots:?}"
    );
}

#[test]
fn loadout_apply_refuses_a_locked_instance_and_changes_nothing() {
    let (_tmp, data_dir) = temp_data_dir();
    create_minimal_instance(&data_dir, "loadout-locked");
    run_agora(&data_dir, &["loadout", "create", "loadout-locked", "all"]);

    // After the profile was saved, lithium was switched off and the instance locked.
    let instance_dir = data_dir.join("instances").join("loadout-locked");
    std::fs::rename(
        instance_dir.join("mods").join("lithium.jar"),
        instance_dir.join("mods").join("lithium.jar.disabled"),
    )
    .unwrap();
    let path = instance_dir.join("instance_manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    manifest["is_locked"] = serde_json::json!(true);
    for entry in manifest["mods"].as_array_mut().unwrap() {
        if entry["filename"] == "lithium.jar" {
            entry["enabled"] = serde_json::json!(false);
        }
    }
    std::fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    let manifest_before = std::fs::read_to_string(&path).unwrap();

    let output = run_agora(&data_dir, &["loadout", "apply", "loadout-locked", "all"]);
    assert!(
        !output.status.success(),
        "apply to a locked instance must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("This instance is locked"),
        "stderr should carry the core lock message:
{stderr}"
    );

    assert!(
        instance_dir
            .join("mods")
            .join("lithium.jar.disabled")
            .exists(),
        "lithium must stay off"
    );
    assert!(!instance_dir.join("mods").join("lithium.jar").exists());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        manifest_before,
        "the manifest must not change"
    );
    let list = run_agora_json(&data_dir, &["snapshots", "list", "loadout-locked"]);
    let snapshots: Vec<serde_json::Value> =
        serde_json::from_value(assert_json_stdout(&list)).unwrap();
    assert!(snapshots.is_empty(), "a refused apply takes no undo point");
}

/// An instance whose profile was saved with sodium and lithium, then gained
/// `newer.jar` (on) after the save, so the profile does not list it.
fn loadout_instance_with_an_unlisted_mod(data_dir: &std::path::Path, id: &str) {
    create_loader_instance(data_dir, id, &["sodium.jar", "lithium.jar"]);
    run_agora(data_dir, &["loadout", "create", id, "both"]);
    create_loader_instance(data_dir, id, &["sodium.jar", "lithium.jar", "newer.jar"]);
}

#[test]
fn loadout_update_refuses_without_a_terminal_unless_yes_and_changes_nothing() {
    let (_tmp, data_dir) = temp_data_dir();
    loadout_instance_with_an_unlisted_mod(&data_dir, "loadout-update-refused");
    let profile = data_dir
        .join("instances")
        .join("loadout-update-refused")
        .join(".agora_loadouts")
        .join("both.json");
    let before = std::fs::read_to_string(&profile).unwrap();

    // Tests have no terminal, so nobody can be asked: the changes are named and
    // the command refuses.
    let output = run_agora(
        &data_dir,
        &["loadout", "update", "loadout-update-refused", "both"],
    );
    assert!(!output.status.success(), "update without --yes must refuse");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("add newer.jar (on)"), "stderr: {stderr}");
    assert!(stderr.contains("--yes"), "stderr: {stderr}");
    assert_eq!(
        std::fs::read_to_string(&profile).unwrap(),
        before,
        "a refused update leaves the profile as it was"
    );
}

#[test]
fn loadout_update_with_yes_records_the_setup_so_apply_stops_asking() {
    let (_tmp, data_dir) = temp_data_dir();
    loadout_instance_with_an_unlisted_mod(&data_dir, "loadout-update-yes");

    let output = run_agora_json(
        &data_dir,
        &["loadout", "update", "loadout-update-yes", "both", "--yes"],
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["status"], "updated", "{parsed}");
    assert_eq!(parsed["changes"][0]["filename"], "newer.jar", "{parsed}");

    // Apply no longer needs --others: the profile now lists newer.jar.
    let apply = run_agora(
        &data_dir,
        &["loadout", "apply", "loadout-update-yes", "both"],
    );
    assert!(
        apply.status.success(),
        "apply after an update should not ask: {}",
        String::from_utf8_lossy(&apply.stderr)
    );

    // And a second update with nothing new says so and writes nothing.
    let again = run_agora(
        &data_dir,
        &["loadout", "update", "loadout-update-yes", "both", "--yes"],
    );
    assert!(again.status.success());
    assert!(String::from_utf8_lossy(&again.stdout).contains("Nothing to save"));
}

#[test]
fn loadout_apply_refuses_unlisted_mods_without_a_choice_and_changes_nothing() {
    let (_tmp, data_dir) = temp_data_dir();
    loadout_instance_with_an_unlisted_mod(&data_dir, "loadout-unlisted");
    let mods = data_dir
        .join("instances")
        .join("loadout-unlisted")
        .join("mods");

    let output = run_agora(&data_dir, &["loadout", "apply", "loadout-unlisted", "both"]);
    assert!(
        !output.status.success(),
        "apply must not choose for the user when mods are unlisted"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("newer.jar"),
        "stderr should name the mod:\n{stderr}"
    );
    assert!(
        stderr.contains("--others"),
        "stderr should name the flag:\n{stderr}"
    );
    assert!(
        mods.join("newer.jar").exists(),
        "a refused apply changes nothing"
    );

    let list = run_agora_json(&data_dir, &["snapshots", "list", "loadout-unlisted"]);
    let snapshots: Vec<serde_json::Value> =
        serde_json::from_value(assert_json_stdout(&list)).unwrap();
    assert!(snapshots.is_empty(), "a refused apply takes no undo point");
}

#[test]
fn loadout_apply_others_off_switches_the_unlisted_mod_off() {
    let (_tmp, data_dir) = temp_data_dir();
    loadout_instance_with_an_unlisted_mod(&data_dir, "loadout-off");
    let mods = data_dir.join("instances").join("loadout-off").join("mods");

    let output = run_agora(
        &data_dir,
        &["loadout", "apply", "loadout-off", "both", "--others", "off"],
    );
    assert!(
        output.status.success(),
        "apply --others off should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        mods.join("newer.jar.disabled").exists(),
        "off switches it off"
    );
    assert!(mods.join("sodium.jar").exists());
    assert!(mods.join("lithium.jar").exists());
}

#[test]
fn loadout_apply_others_keep_leaves_the_unlisted_mod_on() {
    let (_tmp, data_dir) = temp_data_dir();
    loadout_instance_with_an_unlisted_mod(&data_dir, "loadout-keep");
    let mods = data_dir.join("instances").join("loadout-keep").join("mods");

    let output = run_agora(
        &data_dir,
        &[
            "loadout",
            "apply",
            "loadout-keep",
            "both",
            "--others",
            "keep",
        ],
    );
    assert!(
        output.status.success(),
        "apply --others keep should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(mods.join("newer.jar").exists(), "keep leaves it on");
    assert!(!mods.join("newer.jar.disabled").exists());
}

#[test]
fn loadout_apply_rejects_an_unknown_others_value() {
    let (_tmp, data_dir) = temp_data_dir();
    loadout_instance_with_an_unlisted_mod(&data_dir, "loadout-bad-choice");

    let output = run_agora(
        &data_dir,
        &[
            "loadout",
            "apply",
            "loadout-bad-choice",
            "both",
            "--others",
            "maybe",
        ],
    );
    assert!(
        !output.status.success(),
        "a value that is not keep or off is refused"
    );
    let mods = data_dir
        .join("instances")
        .join("loadout-bad-choice")
        .join("mods");
    assert!(mods.join("newer.jar").exists());
}

#[test]
fn loadout_delete_succeeds() {
    let (_tmp, data_dir) = temp_data_dir();
    create_minimal_instance(&data_dir, "loadout-del");

    run_agora(&data_dir, &["loadout", "create", "loadout-del", "todelete"]);
    let output = run_agora(&data_dir, &["loadout", "delete", "loadout-del", "todelete"]);
    assert!(
        output.status.success(),
        "loadout delete should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Deleted"),
        "stdout should contain 'Deleted':\n{stdout}"
    );

    let list = run_agora_json(&data_dir, &["loadout", "list", "loadout-del"]);
    let parsed = assert_json_stdout(&list);
    let profiles: Vec<serde_json::Value> = serde_json::from_value(parsed).unwrap();
    assert!(profiles.is_empty(), "no profiles after delete");
}

#[test]
fn loadout_delete_json_output() {
    let (_tmp, data_dir) = temp_data_dir();
    create_minimal_instance(&data_dir, "loadout-del-json");

    run_agora(
        &data_dir,
        &["loadout", "create", "loadout-del-json", "prof"],
    );
    let output = run_agora_json(
        &data_dir,
        &["loadout", "delete", "loadout-del-json", "prof"],
    );
    assert!(
        output.status.success(),
        "loadout delete --json should succeed"
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["status"], "deleted");
}

// ---------------------------------------------------------------------------
// Lockfile — export / verify / import / repair
// ---------------------------------------------------------------------------

/// Create an instance via DB upsert + manifest for testing lockfile/export commands
/// that require a non-empty loader_version.
fn create_loader_instance(data_dir: &std::path::Path, id: &str, mod_names: &[&str]) {
    // Ensure the local state DB exists and has the schema.
    let db_path = data_dir.join("local_state.db");
    if !db_path.exists() {
        agora_core::db::init_local_state_db(&db_path).unwrap();
    }
    let instance_dir = data_dir.join("instances").join(id);
    std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
    for m in mod_names {
        std::fs::write(instance_dir.join("mods").join(m), m.as_bytes()).unwrap();
    }
    let manifest = serde_json::json!({
        "instance_id": id,
        "name": id,
        "minecraft_version": "1.21",
        "loader": "fabric",
        "loader_version": "0.16.0",
        "is_locked": false,
        "mods": mod_names.iter().map(|m| {
            serde_json::json!({
                "filename": m,
                "source": "manual",
                "version": null,
                "sha256": "0".repeat(64),
                "installed_at": "2024-01-01T00:00:00Z",
                "java_packages": [],
                "modrinth_id": null,
                "registry_id": null,
                "mod_jar_id": null,
                "provided_mod_ids": [],
                "enabled": true,
                "content_type": "mod",
                "depends_on": [],
                "optional_deps": [],
                "incompatible_deps": [],
                "source_url": null,
            })
        }).collect::<Vec<_>>(),
        "resourcepacks": [],
        "shaders": [],
        "datapacks": [],
        "worlds": [],
        "user_preferences": {}
    });
    std::fs::write(
        instance_dir.join("instance_manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    agora_core::db::upsert_instance(
        &conn,
        &agora_core::models::InstanceRow {
            instance_id: id.to_string(),
            name: id.to_string(),
            minecraft_version: "1.21".to_string(),
            loader: "fabric".into(),
            loader_version: "0.16.0".into(),
            is_modpack: false,
            is_locked: false,
            last_launched_at: None,
            jvm_memory_mb: 4096,
            jvm_memory_mode: "manual".into(),
            jvm_gc: "g1gc".into(),
            jvm_custom_args: String::new(),
            jvm_always_pre_touch: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            java_path: None,
            java_incompatible_override: false,
            icon_path: None,
            launch_mode_override: "auto".into(),
            import_source: None,
            total_play_ms: 0,
        },
    )
    .unwrap();
}

#[test]
fn lockfile_export_succeeds() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "lf-export", &["sodium.jar", "lithium.jar"]);

    let output = run_agora(&data_dir, &["lockfile", "export", "lf-export"]);
    assert!(
        output.status.success(),
        "lockfile export should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // With no --out flag, lockfile content is printed to stdout
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout should be valid lockfile JSON: {e}\n{stdout}"));
    assert_eq!(parsed["schemaVersion"], 1);
    assert_eq!(parsed["instance"]["name"], "lf-export");
    assert!(
        parsed.get("artifacts").and_then(|a| a.as_array()).is_some(),
        "lockfile should have artifacts array"
    );
}

#[test]
fn lockfile_export_to_file() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "lf-file", &["sodium.jar"]);

    let out_path = data_dir.join("lockfile.json");
    let output = run_agora(
        &data_dir,
        &[
            "lockfile",
            "export",
            "lf-file",
            "--out",
            &out_path.to_string_lossy(),
        ],
    );
    assert!(
        output.status.success(),
        "lockfile export --out should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Exported lockfile"),
        "stdout should mention 'Exported lockfile':\n{stdout}"
    );

    assert!(out_path.exists(), "lockfile file should exist");
    let text = std::fs::read_to_string(&out_path).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&text).expect("lockfile file should be valid JSON");
    assert_eq!(parsed["schemaVersion"], 1);
}

#[test]
fn lockfile_export_json_output() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "lf-json", &["sodium.jar"]);

    let out_path = data_dir.join("lockfile-out.json");
    let output = run_agora_json(
        &data_dir,
        &[
            "lockfile",
            "export",
            "lf-json",
            "--out",
            &out_path.to_string_lossy(),
        ],
    );
    assert!(
        output.status.success(),
        "lockfile export --json should succeed"
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["status"], "exported");
    assert!(
        parsed["path"]
            .as_str()
            .unwrap_or("")
            .contains("lockfile-out.json"),
        "exported path should be mentioned in JSON output"
    );
}

#[test]
fn lockfile_vanilla_export_then_verify_passes() {
    let (_tmp, data_dir) = temp_data_dir();
    run_agora(&data_dir, &["paths"]);
    create_vanilla_instance(&data_dir, "lf-vanilla", "1.21");

    let lf_path = data_dir.join("lf-vanilla.json");
    let export = run_agora(
        &data_dir,
        &[
            "lockfile",
            "export",
            "lf-vanilla",
            "--out",
            &lf_path.to_string_lossy(),
        ],
    );
    assert!(
        export.status.success(),
        "vanilla lockfile export should succeed:\n{}",
        String::from_utf8_lossy(&export.stderr),
    );

    let text = std::fs::read_to_string(&lf_path).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&text).expect("vanilla lockfile should be valid JSON");
    assert_eq!(parsed["instance"]["loader"], "vanilla");
    assert_eq!(parsed["instance"]["loaderVersion"], "");

    let verify = run_agora(
        &data_dir,
        &["lockfile", "verify", &lf_path.to_string_lossy()],
    );
    assert!(
        verify.status.success(),
        "vanilla export must immediately pass lockfile verify:\n{}",
        String::from_utf8_lossy(&verify.stderr),
    );
    let stdout = String::from_utf8_lossy(&verify.stdout);
    assert!(
        stdout.contains("Lockfile is valid") || stdout.contains("valid"),
        "stdout should confirm validity:\n{stdout}"
    );
}

#[test]
fn lockfile_verify_valid() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "lf-verify", &["sodium.jar"]);
    let lf_path = data_dir.join("lf-verify.json");
    run_agora(
        &data_dir,
        &[
            "lockfile",
            "export",
            "lf-verify",
            "--out",
            &lf_path.to_string_lossy(),
        ],
    );

    let output = run_agora(
        &data_dir,
        &["lockfile", "verify", &lf_path.to_string_lossy()],
    );
    assert!(
        output.status.success(),
        "lockfile verify should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Lockfile is valid") | stdout.contains("valid"),
        "stdout should confirm validity:\n{stdout}"
    );
}

#[test]
fn lockfile_verify_invalid_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let bad_path = data_dir.join("bad-lockfile.json");
    std::fs::write(&bad_path, r#"{"schemaVersion":999,"instance":{"name":"","minecraftVersion":"","loader":"","loaderVersion":""},"artifacts":[],"loader":{},"manifestSha256":"","configPolicy":{},"contentHash":""}"#)
        .unwrap();

    let output = run_agora(
        &data_dir,
        &["lockfile", "verify", &bad_path.to_string_lossy()],
    );
    assert!(
        !output.status.success(),
        "lockfile verify should fail for invalid lockfile"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid") || stderr.contains("Unsupported"),
        "stderr should mention 'invalid' or 'Unsupported':\n{stderr}"
    );
}

#[test]
fn lockfile_verify_json_output() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "lf-verify-json", &["sodium.jar"]);

    let lf_path = data_dir.join("lf-verify-json.json");
    run_agora(
        &data_dir,
        &[
            "lockfile",
            "export",
            "lf-verify-json",
            "--out",
            &lf_path.to_string_lossy(),
        ],
    );

    let output = run_agora_json(
        &data_dir,
        &["lockfile", "verify", &lf_path.to_string_lossy()],
    );
    assert!(
        output.status.success(),
        "lockfile verify --json should succeed"
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["status"], "valid");
    assert!(parsed.get("schemaVersion").is_some());
}

#[test]
fn lockfile_repair_succeeds() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "lf-repair", &["sodium.jar"]);

    let out_path = data_dir.join("repaired.json");
    let output = run_agora(
        &data_dir,
        &[
            "lockfile",
            "repair",
            "lf-repair",
            "--out",
            &out_path.to_string_lossy(),
        ],
    );
    assert!(
        output.status.success(),
        "lockfile repair should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Repaired"),
        "stdout should contain 'Repaired':\n{stdout}"
    );
    assert!(out_path.exists(), "repaired lockfile should exist");
    let text = std::fs::read_to_string(&out_path).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&text).expect("repaired lockfile should be valid JSON");
    assert_eq!(parsed["schemaVersion"], 1);
}

#[test]
fn lockfile_repair_json_output() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "lf-repair-json", &["sodium.jar"]);

    let out_path = data_dir.join("repaired.json");
    let output = run_agora_json(
        &data_dir,
        &[
            "lockfile",
            "repair",
            "lf-repair-json",
            "--out",
            &out_path.to_string_lossy(),
        ],
    );
    assert!(
        output.status.success(),
        "lockfile repair --json should succeed"
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["status"], "repaired");
}

#[test]
fn lockfile_import_detects_drift() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "lf-import", &["sodium.jar"]);

    // Export the lockfile
    let lf_path = data_dir.join("drift-lockfile.json");
    run_agora(
        &data_dir,
        &[
            "lockfile",
            "export",
            "lf-import",
            "--out",
            &lf_path.to_string_lossy(),
        ],
    );

    // Add a new mod after export (creates drift)
    std::fs::write(
        data_dir.join("instances/lf-import/mods/extra.jar"),
        b"extra",
    )
    .unwrap();

    let output = run_agora(
        &data_dir,
        &[
            "lockfile",
            "import",
            &lf_path.to_string_lossy(),
            "lf-import",
        ],
    );
    assert!(
        output.status.success(),
        "lockfile import should succeed (drift is informational):\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Drift") || stdout.contains("differences"),
        "stdout should mention drift:\n{stdout}"
    );
}

#[test]
fn lockfile_import_without_drift_reports_in_sync() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "lf-sync", &["sodium.jar"]);

    let lf_path = data_dir.join("sync-lockfile.json");
    run_agora(
        &data_dir,
        &[
            "lockfile",
            "export",
            "lf-sync",
            "--out",
            &lf_path.to_string_lossy(),
        ],
    );

    let output = run_agora(
        &data_dir,
        &["lockfile", "import", &lf_path.to_string_lossy(), "lf-sync"],
    );
    assert!(
        output.status.success(),
        "lockfile import on in-sync instance should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("already in sync"),
        "stdout should mention 'already in sync':\n{stdout}"
    );
}

#[test]
fn lockfile_nonexistent_instance_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["lockfile", "export", "no-such-instance"]);
    assert!(
        !output.status.success(),
        "lockfile export on nonexistent instance should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "stderr should mention 'not found':\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// User-decision-required — stable exit code 71 and JSON envelope
// ---------------------------------------------------------------------------

#[test]
fn user_decision_required_json_envelope_has_exit_code() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let instance_src = src.path().join("udr-test");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();
    std::fs::write(instance_src.join("mods").join("a.jar"), b"mod").unwrap();
    run_agora(&data_dir, &["import", &instance_src.to_string_lossy()]);

    // The mod remove pipeline on a nonexistent mod produces blocking errors
    // (not UserDecisionRequired directly), but the JSON envelope should still
    // contain an exitCode > 0 field.
    let output = run_agora_json(&data_dir, &["mod", "remove", "nonexistent-mod", "udr-test"]);
    assert!(!output.status.success(), "mod remove should fail");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let json_start = stderr.rfind('{').unwrap_or_else(|| {
        panic!("stderr must contain JSON envelope:\n{stderr}");
    });
    let json_end = stderr[json_start..]
        .rfind('}')
        .map(|i| json_start + i + 1)
        .unwrap_or_else(|| {
            panic!("JSON envelope must have closing '}}':\n{stderr}");
        });
    let json_text = &stderr[json_start..json_end];
    let parsed: serde_json::Value = serde_json::from_str(json_text)
        .unwrap_or_else(|e| panic!("failed to parse JSON envelope: {e}\ntext:\n{json_text}"));

    let exit_code = parsed["exitCode"]
        .as_i64()
        .expect("JSON envelope must contain 'exitCode'");
    assert!(
        exit_code > 0,
        "exitCode must be > 0 on error, got {exit_code}"
    );
    assert!(
        parsed.get("error").is_some(),
        "JSON envelope must contain 'error' field"
    );
}

#[test]
fn user_decision_required_human_output_mentions_blocked() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let instance_src = src.path().join("udr-human");
    std::fs::create_dir_all(instance_src.join("mods")).unwrap();
    std::fs::write(instance_src.join("mods").join("a.jar"), b"mod").unwrap();
    run_agora(&data_dir, &["import", &instance_src.to_string_lossy()]);

    let output = run_agora(
        &data_dir,
        &["mod", "remove", "nonexistent-mod", "udr-human"],
    );
    assert!(!output.status.success(), "mod remove must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not installed")
            || stderr.contains("blocked")
            || stderr.contains("unresolved"),
        "stderr should mention 'not installed' or 'blocked':\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// migrate-data command
// ---------------------------------------------------------------------------

#[test]
fn migrate_data_from_nonexistent_path_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    let bad_path = std::path::Path::new("Z:\\no-such-path\\agora-data");
    let output = run_agora(
        &data_dir,
        &["migrate-data", "--from", &bad_path.to_string_lossy()],
    );
    assert!(
        !output.status.success(),
        "migrate-data from nonexistent path should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not exist"),
        "stderr should mention 'does not exist':\n{stderr}"
    );
}

#[test]
fn migrate_data_dry_run_shows_inventory() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let src_path = src.path().join("old-data");
    std::fs::create_dir_all(src_path.join("instances/my-inst/mods")).unwrap();
    std::fs::write(src_path.join("local_state.db"), b"old state").unwrap();
    std::fs::write(
        src_path.join("instances/my-inst/instance_manifest.json"),
        br#"{"instance_id":"my-inst"}"#,
    )
    .unwrap();

    let output = run_agora(
        &data_dir,
        &["migrate-data", "--from", &src_path.to_string_lossy()],
    );
    assert!(
        output.status.success(),
        "dry-run should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Dry-run"),
        "stdout should say 'Dry-run':\n{stdout}"
    );
    assert!(
        stdout.contains("my-inst"),
        "stdout should list instances:\n{stdout}"
    );
    assert!(
        stdout.contains("--yes"),
        "stdout should mention --yes flag:\n{stdout}"
    );
}

#[test]
fn migrate_data_json_dry_run() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let src_path = src.path().join("old-data");
    std::fs::create_dir_all(&src_path).unwrap();
    std::fs::write(src_path.join("test.txt"), b"hello").unwrap();

    let output = run_agora_json(
        &data_dir,
        &["migrate-data", "--from", &src_path.to_string_lossy()],
    );
    assert!(
        output.status.success(),
        "JSON dry-run should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["status"], "dry-run");
    assert!(parsed.get("sourceInventory").is_some());
    assert!(parsed.get("conflicts").is_some());
}

#[test]
fn migrate_data_execute_moves_files() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let src_path = src.path().join("old-data");
    std::fs::create_dir_all(src_path.join("instances/my-inst/mods")).unwrap();
    std::fs::write(src_path.join("local_state.db"), b"old state").unwrap();
    std::fs::write(src_path.join("registry.db"), b"old registry").unwrap();
    std::fs::write(
        src_path.join("instances/my-inst/instance_manifest.json"),
        br#"{"name":"my-inst"}"#,
    )
    .unwrap();
    std::fs::write(
        src_path.join("instances/my-inst/mods/sodium.jar"),
        b"sodium",
    )
    .unwrap();

    let output = run_agora(
        &data_dir,
        &[
            "migrate-data",
            "--from",
            &src_path.to_string_lossy(),
            "--yes",
        ],
    );
    assert!(
        output.status.success(),
        "migrate-data --yes should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Migration complete"),
        "stdout should contain 'Migration complete':\n{stdout}"
    );
    assert!(
        stdout.contains("my-inst"),
        "stdout should mention migrated instances:\n{stdout}"
    );

    // Verify files are at the destination
    assert!(
        data_dir.join("local_state.db").exists(),
        "local_state.db should be migrated"
    );
    assert!(
        data_dir
            .join("instances/my-inst/instance_manifest.json")
            .exists(),
        "instance manifest should be migrated"
    );
    assert!(
        data_dir.join("instances/my-inst/mods/sodium.jar").exists(),
        "mod file should be migrated"
    );

    // Verify backup was created
    let stdout_str = stdout.to_string();
    assert!(
        stdout_str.contains("Backup:"),
        "stdout should mention backup:\n{stdout}"
    );

    // Verify content integrity
    assert_eq!(
        std::fs::read(data_dir.join("local_state.db")).unwrap(),
        b"old state"
    );
    assert_eq!(
        std::fs::read(data_dir.join("instances/my-inst/mods/sodium.jar")).unwrap(),
        b"sodium"
    );
}

#[test]
fn migrate_data_json_execute() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let src_path = src.path().join("old-data");
    std::fs::create_dir_all(&src_path).unwrap();
    std::fs::write(src_path.join("test.txt"), b"data").unwrap();

    let output = run_agora_json(
        &data_dir,
        &[
            "migrate-data",
            "--from",
            &src_path.to_string_lossy(),
            "--yes",
        ],
    );
    assert!(
        output.status.success(),
        "JSON migrate-data --yes should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["files_migrated"], 1);
    assert!(parsed.get("backup_path").is_some());
    assert!(!parsed["backup_path"].as_str().unwrap_or("").is_empty());
}

#[test]
fn migrate_data_conflict_refused() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let src_path = src.path().join("old-data");
    std::fs::create_dir_all(&src_path).unwrap();
    std::fs::write(src_path.join("local_state.db"), b"source db").unwrap();

    // Pre-create a local_state.db at the destination
    std::fs::write(data_dir.join("local_state.db"), b"dest db").unwrap();

    let output = run_agora(
        &data_dir,
        &[
            "migrate-data",
            "--from",
            &src_path.to_string_lossy(),
            "--yes",
        ],
    );
    assert!(
        !output.status.success(),
        "migrate-data with conflicting db should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("blocked") || stderr.contains("conflict") || stderr.contains("conflict"),
        "stderr should mention conflict:\n{stderr}"
    );
}

#[test]
fn migrate_data_empty_source_succeeds_dry_run() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let src_path = src.path().join("empty-data");
    std::fs::create_dir_all(&src_path).unwrap();

    let output = run_agora(
        &data_dir,
        &["migrate-data", "--from", &src_path.to_string_lossy()],
    );
    assert!(
        output.status.success(),
        "dry-run on empty source should succeed"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Dry-run"),
        "stdout should say Dry-run:\n{stdout}"
    );
}

#[test]
fn migrate_data_empty_source_execute_succeeds() {
    let (_tmp, data_dir) = temp_data_dir();
    let src = tempdir();
    let src_path = src.path().join("empty-data");
    std::fs::create_dir_all(&src_path).unwrap();

    let output = run_agora(
        &data_dir,
        &[
            "migrate-data",
            "--from",
            &src_path.to_string_lossy(),
            "--yes",
        ],
    );
    assert!(
        output.status.success(),
        "execute on empty source should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

// ---------------------------------------------------------------------------
// Plugin management
// ---------------------------------------------------------------------------

/// `--yes` is the scripting escape hatch for capability consent, and it has to
/// parse wherever someone naturally writes it. Clap would otherwise accept it
/// only before the `package` / `development` subcommand, which is not the form
/// `docs/CLI.md` shows and not the form a person reaches for.
#[test]
fn the_consent_flag_parses_on_either_side_of_the_source_subcommand() {
    let example =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/dashboard");
    let example = example.to_string_lossy().to_string();

    for args in [
        vec![
            "plugin",
            "install",
            "--yes",
            "development",
            example.as_str(),
        ],
        vec![
            "plugin",
            "install",
            "development",
            example.as_str(),
            "--yes",
        ],
    ] {
        let data_dir = tempdir();
        let output = run_agora(data_dir.path(), &args);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "`{}` failed:
stdout: {stdout}
stderr: {stderr}",
            args.join(" ")
        );
        assert!(
            stdout.contains("agora.dashboard"),
            "`{}` did not report the installed plugin: {stdout}",
            args.join(" ")
        );
    }
}

/// Installing something that wants capabilities must not succeed just because
/// nobody was there to say no. The refusal comes from core, not from the CLI.
#[test]
fn a_non_interactive_install_without_consent_is_refused() {
    let data_dir = tempdir();
    let example =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/dashboard");
    let example = example.to_string_lossy().to_string();

    let output = run_agora(
        data_dir.path(),
        &["plugin", "install", "development", example.as_str()],
    );
    assert!(
        !output.status.success(),
        "an unconsented install should fail"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("--yes"),
        "the refusal should tell a script how to proceed: {combined}"
    );

    // And nothing was installed.
    let listed = run_agora(data_dir.path(), &["plugin", "list"]);
    assert!(!String::from_utf8_lossy(&listed.stdout).contains("agora.dashboard"));
}

/// Removing a plugin keeps its data unless asked otherwise, and says which it did.
#[test]
fn removal_reports_whether_stored_data_was_kept() {
    let data_dir = tempdir();
    let example =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/dashboard");
    let example = example.to_string_lossy().to_string();

    run_agora(
        data_dir.path(),
        &[
            "plugin",
            "install",
            "--yes",
            "development",
            example.as_str(),
        ],
    );
    let output = run_agora(data_dir.path(), &["plugin", "remove", "agora.dashboard"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(
        stdout.to_lowercase().contains("kept"),
        "removal should state that data was kept: {stdout}"
    );
}

#[test]
fn games_base_cli_list_verify_remove() {
    let (_tmp, data_dir) = temp_data_dir();

    // 1. List when empty
    let output = run_agora(&data_dir, &["games", "base", "list"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No pinned bases found"));

    let output_json = run_agora_json(&data_dir, &["games", "base", "list"]);
    assert!(output_json.status.success());
    let json_val: serde_json::Value =
        serde_json::from_slice(&output_json.stdout).expect("valid json array");
    assert_eq!(json_val, serde_json::json!([]));

    // 2. Verify nonexistent base -> error
    let output_ver = run_agora(&data_dir, &["games", "base", "verify", "nonexistent-base"]);
    assert!(!output_ver.status.success());

    // 3. Remove nonexistent base -> error
    let output_rem = run_agora(&data_dir, &["games", "base", "remove", "nonexistent-base"]);
    assert!(!output_rem.status.success());

    // 4. Build with nonexistent install -> error
    let output_build = run_agora(&data_dir, &["games", "base", "build", "steam:nonexistent"]);
    assert!(!output_build.status.success());

    // 5. Launch with nonexistent base -> error
    let output_launch = run_agora(&data_dir, &["games", "launch", "nonexistent-base"]);
    assert!(!output_launch.status.success());
}

#[test]
fn games_base_cli_lists_damaged_records_with_their_reason() {
    let (_tmp, data_dir) = temp_data_dir();
    let manifests = data_dir.join("bases").join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    let base_id = "skyrim-se_steam_2.0.0_nobuild";
    std::fs::write(manifests.join(format!("{base_id}.json")), "{ not json").unwrap();

    let output = run_agora(&data_dir, &["games", "base", "list"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(base_id),
        "the damaged base is listed: {stdout}"
    );
    assert!(
        stdout.contains("damaged"),
        "and says it is damaged: {stdout}"
    );
    assert!(
        stdout.contains("is not a valid base record"),
        "with the reason: {stdout}"
    );

    let json = run_agora_json(&data_dir, &["games", "base", "list"]);
    assert!(json.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("valid json array");
    assert_eq!(value[0]["state"], "damaged");
    assert_eq!(value[0]["base_id"], base_id);
    assert!(value[0]["error"]
        .as_str()
        .unwrap()
        .contains("is not a valid base record"));
}

#[test]
fn games_content_cli_lifecycle() {
    let (_tmp, data_dir) = temp_data_dir();

    // 1. List when empty
    let output = run_agora(&data_dir, &["games", "content", "list"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No content items found"));

    let output_json = run_agora_json(&data_dir, &["games", "content", "list"]);
    assert!(output_json.status.success());
    let json_val: serde_json::Value =
        serde_json::from_slice(&output_json.stdout).expect("valid json array");
    assert_eq!(json_val, serde_json::json!([]));

    // 2. Add folder
    let mod_dir = tempfile::tempdir().unwrap();
    std::fs::write(mod_dir.path().join("readme.txt"), b"mod content").unwrap();
    let mod_dir_str = mod_dir.path().to_str().unwrap();

    let output_add = run_agora(
        &data_dir,
        &["games", "content", "add", mod_dir_str, "--name", "test-mod"],
    );
    assert!(output_add.status.success());
    let add_stdout = String::from_utf8_lossy(&output_add.stdout);
    assert!(add_stdout.contains("Added item"));

    // 3. List
    let output_list = run_agora(&data_dir, &["games", "content", "list"]);
    assert!(output_list.status.success());
    let list_stdout = String::from_utf8_lossy(&output_list.stdout);
    assert!(list_stdout.contains("test-mod"));

    let output_list_json = run_agora_json(&data_dir, &["games", "content", "list"]);
    let list_json: serde_json::Value =
        serde_json::from_slice(&output_list_json.stdout).expect("valid json list");
    let item_id = list_json[0]["item_id"].as_str().unwrap().to_string();
    let prefix = &item_id[..8];

    // 4. Show
    let output_show = run_agora(&data_dir, &["games", "content", "show", prefix]);
    assert!(output_show.status.success());
    let show_stdout = String::from_utf8_lossy(&output_show.stdout);
    assert!(show_stdout.contains("readme.txt"));

    // 5. Verify clean
    let output_verify = run_agora(&data_dir, &["games", "content", "verify", prefix]);
    assert!(output_verify.status.success());
    let verify_stdout = String::from_utf8_lossy(&output_verify.stdout);
    assert!(verify_stdout.contains("verified clean"));

    // 6. Remove
    let output_remove = run_agora(&data_dir, &["games", "content", "remove", prefix]);
    assert!(output_remove.status.success());
    let remove_stdout = String::from_utf8_lossy(&output_remove.stdout);
    assert!(remove_stdout.contains("removed"));

    // 7. Verify nonexistent fails
    let output_ver_err = run_agora(&data_dir, &["games", "content", "verify", prefix]);
    assert!(!output_ver_err.status.success());

    // 8. Remove nonexistent fails
    let output_rem_err = run_agora(&data_dir, &["games", "content", "remove", prefix]);
    assert!(!output_rem_err.status.success());
}

#[test]
fn games_content_fomod_cli_show_and_install() {
    let (_tmp, data_dir) = temp_data_dir();
    let mod_dir = tempfile::tempdir().unwrap();
    let root = mod_dir.path();
    std::fs::create_dir_all(root.join("fomod")).unwrap();
    std::fs::create_dir_all(root.join("Core")).unwrap();
    std::fs::create_dir_all(root.join("Heavy")).unwrap();
    std::fs::create_dir_all(root.join("Light")).unwrap();
    std::fs::write(root.join("Core").join("core.esp"), b"core").unwrap();
    std::fs::write(root.join("Heavy").join("armor.nif"), b"heavy").unwrap();
    std::fs::write(root.join("Light").join("armor.nif"), b"light").unwrap();
    std::fs::write(
        root.join("fomod").join("ModuleConfig.xml"),
        r#"<config>
  <moduleName>CLI Mod</moduleName>
  <requiredInstallFiles><folder source="Core" destination=""/></requiredInstallFiles>
  <installSteps order="Explicit"><installStep name="Armor"><optionalFileGroups>
    <group name="Weight" type="SelectExactlyOne"><plugins>
      <plugin name="Heavy"><files><folder source="Heavy" destination="meshes"/></files>
        <typeDescriptor><type name="Recommended"/></typeDescriptor></plugin>
      <plugin name="Light"><files><folder source="Light" destination="meshes"/></files></plugin>
    </plugins></group>
  </optionalFileGroups></installStep></installSteps>
</config>"#,
    )
    .unwrap();

    let add = run_agora_json(
        &data_dir,
        &[
            "games",
            "content",
            "add",
            root.to_str().unwrap(),
            "--name",
            "cli-mod",
        ],
    );
    assert!(add.status.success());
    let added: serde_json::Value = serde_json::from_slice(&add.stdout).unwrap();
    let item_id = added["item"]["item_id"].as_str().unwrap().to_string();
    let prefix = &item_id[..12];

    // show: human and JSON.
    let show = run_agora(&data_dir, &["games", "content", "fomod", "show", prefix]);
    assert!(show.status.success());
    let text = String::from_utf8_lossy(&show.stdout);
    assert!(text.contains("CLI Mod"));
    assert!(text.contains("Group: Weight (select exactly one)"));
    assert!(text.contains("--choose \"Armor/Weight/Heavy\""));
    let show_json = run_agora_json(&data_dir, &["games", "content", "fomod", "show", prefix]);
    let installer: serde_json::Value = serde_json::from_slice(&show_json.stdout).unwrap();
    assert_eq!(
        installer["steps"][0]["groups"][0]["group_type"],
        "select_exactly_one"
    );

    // install without choices names the group and its rule.
    let none = run_agora(&data_dir, &["games", "content", "fomod", "install", prefix]);
    assert!(!none.status.success());
    let err = String::from_utf8_lossy(&none.stderr);
    assert!(
        err.contains("Armor/Weight") && err.contains("select exactly one"),
        "{err}"
    );

    // An unknown option is an error too.
    let bad = run_agora(
        &data_dir,
        &[
            "games",
            "content",
            "fomod",
            "install",
            prefix,
            "--choose",
            "Armor/Weight/Nope",
        ],
    );
    assert!(!bad.status.success());

    // --choose installs one option; --defaults picks the recommended one.
    let light = run_agora_json(
        &data_dir,
        &[
            "games",
            "content",
            "fomod",
            "install",
            prefix,
            "--choose",
            "armor/weight/light",
        ],
    );
    assert!(
        light.status.success(),
        "{}",
        String::from_utf8_lossy(&light.stderr)
    );
    let light: serde_json::Value = serde_json::from_slice(&light.stdout).unwrap();
    let dests: Vec<&str> = light["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["destination"].as_str().unwrap())
        .collect();
    assert_eq!(dests, ["core.esp", "meshes/armor.nif"]);

    let heavy = run_agora_json(
        &data_dir,
        &["games", "content", "fomod", "install", prefix, "--defaults"],
    );
    assert!(heavy.status.success());
    let heavy: serde_json::Value = serde_json::from_slice(&heavy.stdout).unwrap();
    assert_eq!(heavy["choices"][0]["plugins"][0], "Heavy");
    assert_ne!(heavy["item_id"], light["item_id"]);

    // Both derived items are listed with their provenance, and verify clean.
    let list = run_agora(&data_dir, &["games", "content", "list"]);
    let list_text = String::from_utf8_lossy(&list.stdout);
    assert!(
        list_text.contains("CLI Mod (FOMOD)") && list_text.contains("fomod:"),
        "{list_text}"
    );
    let verify = run_agora(&data_dir, &["games", "content", "verify", "--full"]);
    assert!(verify.status.success());
}

#[test]
fn games_instance_content_add_of_a_fomod_archive_exits_71_without_an_error() {
    // A pinned Skyrim SE instance, and a small FOMOD folder in the content store (the folder form
    // of `games content add`, as the fomod test above uses).
    let f = skyrim_fixture("1.6.1170.0", &[]);
    let mod_dir = tempfile::tempdir().unwrap();
    let root = mod_dir.path();
    std::fs::create_dir_all(root.join("fomod")).unwrap();
    std::fs::create_dir_all(root.join("Core")).unwrap();
    std::fs::write(
        root.join("fomod").join("ModuleConfig.xml"),
        r#"<config><moduleName>Tiny FOMOD</moduleName>
  <requiredInstallFiles><folder source="Core" destination=""/></requiredInstallFiles>
</config>"#,
    )
    .unwrap();
    std::fs::write(root.join("Core").join("tiny.esp"), b"plugin").unwrap();

    let add = run_agora_json(
        &f.data_dir,
        &[
            "games",
            "content",
            "add",
            root.to_str().unwrap(),
            "--name",
            "tiny-fomod",
        ],
    );
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let added: serde_json::Value = serde_json::from_slice(&add.stdout).unwrap();
    let item_id = added["item"]["item_id"].as_str().unwrap().to_string();

    // Human output: the guidance on stdout, no "Error:" anywhere, exit 71.
    let human = run_agora(
        &f.data_dir,
        &[
            "games",
            "instance",
            "content",
            "add",
            f.instance_id.as_str(),
            item_id.as_str(),
        ],
    );
    let out = String::from_utf8_lossy(&human.stdout);
    let err = String::from_utf8_lossy(&human.stderr);
    assert_eq!(
        human.status.code(),
        Some(71),
        "stdout: {out}\nstderr: {err}"
    );
    assert!(
        !out.contains("Error:") && !err.contains("Error:"),
        "stdout: {out}\nstderr: {err}"
    );
    assert!(
        out.contains(&format!(
            "agora games content fomod install {item_id} --instance {} --defaults",
            f.instance_id
        )),
        "{out}"
    );

    // JSON: stdout stays empty, and stderr is one JSON envelope with the same exit code.
    let json = run_agora_json(
        &f.data_dir,
        &[
            "games",
            "instance",
            "content",
            "add",
            f.instance_id.as_str(),
            item_id.as_str(),
        ],
    );
    assert_eq!(json.status.code(), Some(71));
    assert!(
        json.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&json.stdout)
    );
    // stderr may carry a context warning before the envelope (the catalog loader line), so the
    // envelope starts at its first brace, as the other envelope tests read it.
    let stderr = String::from_utf8_lossy(&json.stderr);
    let envelope: serde_json::Value = serde_json::from_str(&stderr[stderr.find('{').unwrap()..])
        .expect("stderr ends in one JSON object");
    assert_eq!(envelope["exitCode"], 71);
    assert_eq!(envelope["status"], "needs_installer");
    assert!(envelope["installer_command"]
        .as_str()
        .unwrap()
        .starts_with("agora games content fomod install"));
}

#[test]
fn games_instance_content_and_deploy_cli() {
    let (_tmp, data_dir) = temp_data_dir();

    // 1. Nonexistent instance errors
    let out = run_agora(
        &data_dir,
        &["games", "instance", "content", "list", "no-such-inst"],
    );
    assert!(!out.status.success());

    let out_json = run_agora_json(
        &data_dir,
        &["games", "instance", "content", "list", "no-such-inst"],
    );
    assert!(!out_json.status.success());

    let out_deploy = run_agora(&data_dir, &["games", "instance", "deploy", "no-such-inst"]);
    assert!(!out_deploy.status.success());

    let out_undeploy = run_agora(
        &data_dir,
        &["games", "instance", "undeploy", "no-such-inst"],
    );
    assert!(!out_undeploy.status.success());

    let out_set = run_agora(
        &data_dir,
        &[
            "games",
            "instance",
            "set-deployment",
            "no-such-inst",
            "virtual",
        ],
    );
    assert!(!out_set.status.success());

    // A mode name nobody knows is refused before anything is touched.
    let out_bad = run_agora(
        &data_dir,
        &[
            "games",
            "instance",
            "set-deployment",
            "no-such-inst",
            "fastest",
        ],
    );
    assert!(!out_bad.status.success());
    assert!(String::from_utf8_lossy(&out_bad.stderr).contains("unknown deployment"));

    // The plugin list commands fail closed on an instance that is not there.
    for args in [
        &["games", "instance", "plugins", "no-such-inst"][..],
        &["games", "instance", "ini", "no-such-inst"][..],
        &[
            "games",
            "instance",
            "ini",
            "no-such-inst",
            "user/Skyrim.ini",
            "General",
            "k",
        ][..],
        &["games", "instance", "saves", "no-such-inst"][..],
        &["games", "instance", "saves", "no-such-inst", "own"][..],
        &[
            "games",
            "instance",
            "plugins",
            "enable",
            "no-such-inst",
            "A.esp",
        ][..],
        &[
            "games",
            "instance",
            "plugins",
            "disable",
            "no-such-inst",
            "A.esp",
        ][..],
    ] {
        let out = run_agora(&data_dir, args);
        assert!(!out.status.success(), "{args:?} should fail");
        let out_json = run_agora_json(&data_dir, args);
        assert!(!out_json.status.success(), "{args:?} --json should fail");
    }
    // Naming no instance is a usage error, not an empty list.
    let out = run_agora(&data_dir, &["games", "instance", "plugins"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn games_launch_commands_offer_plain_to_skip_a_framework_loader() {
    let (_tmp, data_dir) = temp_data_dir();
    for command in [
        &["games", "launch"][..],
        &["games", "instance", "launch"][..],
    ] {
        let mut args = command.to_vec();
        args.push("--help");
        let out = run_agora(&data_dir, &args);
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("--plain"));
    }
}

#[test]
fn games_user_files_cli_status_and_restore() {
    let (_tmp, data_dir) = temp_data_dir();

    // 1. Status with no sessions should succeed and be empty
    let out = run_agora(&data_dir, &["games", "user-files", "status"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("No user-file swap sessions"));

    let out_json = run_agora_json(&data_dir, &["games", "user-files", "status"]);
    assert!(out_json.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out_json.stdout)).unwrap();
    assert_eq!(parsed, serde_json::json!([]));

    // 2. Status with game filter
    let out_game = run_agora(&data_dir, &["games", "user-files", "status", "skyrim-se"]);
    assert!(out_game.status.success());

    // 3. Restore with nonexistent session errors
    let out_restore = run_agora(
        &data_dir,
        &["games", "user-files", "restore", "skyrim-se", "steam"],
    );
    assert!(!out_restore.status.success());

    let out_restore_json = run_agora_json(
        &data_dir,
        &["games", "user-files", "restore", "skyrim-se", "steam"],
    );
    assert!(!out_restore_json.status.success());
}

#[test]
fn games_instance_content_add_thunderstore_and_fallback_to_suggest_placement() {
    let (_tmp, data_dir) = temp_data_dir();

    // 0. Enable plugins
    let out_set = run_agora(&data_dir, &["settings", "set", "plugins_enabled", "true"]);
    assert!(out_set.status.success());

    // 1. Install Valheim tracer plugin so valheim game definition is available
    let valheim_pkg =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/tracers/valheim");
    let valheim_pkg_str = valheim_pkg.to_string_lossy().to_string();
    let out_plugin = run_agora(
        &data_dir,
        &[
            "plugin",
            "install",
            "--yes",
            "development",
            valheim_pkg_str.as_str(),
        ],
    );
    assert!(
        out_plugin.status.success(),
        "failed to install valheim plugin: {}",
        String::from_utf8_lossy(&out_plugin.stderr)
    );

    // 2. Create Valheim instance manifest directly in instances/valheim-test
    let inst_dir = data_dir.join("instances").join("valheim-test");
    std::fs::create_dir_all(&inst_dir).unwrap();
    let manifest_json = serde_json::json!({
        "manifest_version": 3,
        "game": "valheim",
        "instance_id": "valheim-test",
        "name": "Valheim Test",
        "base": {
            "kind": "unpinned",
            "install": "steam:892970",
            "reason": "testing"
        },
        "frameworks": [],
        "layers": []
    });
    std::fs::write(
        inst_dir.join("instance_manifest.json"),
        serde_json::to_string_pretty(&manifest_json).unwrap(),
    )
    .unwrap();

    // 3. Add a non-Thunderstore folder to content store
    let non_ts_dir = tempdir();
    std::fs::write(non_ts_dir.path().join("readme.txt"), b"not a ts package").unwrap();
    let out_add_content = run_agora(
        &data_dir,
        &[
            "games",
            "content",
            "add",
            non_ts_dir.path().to_str().unwrap(),
            "--name",
            "non-ts-mod",
        ],
    );
    assert!(out_add_content.status.success());
    let list_out = run_agora_json(&data_dir, &["games", "content", "list"]);
    let list_json: serde_json::Value = serde_json::from_slice(&list_out.stdout).unwrap();
    let non_ts_item_id = list_json[0]["item_id"].as_str().unwrap().to_string();

    // 4. Try `games instance content add` without flags on non-Thunderstore item:
    // It falls back to suggest_placement, which fails with Suggestion::Unknown
    let out_place_unknown = run_agora(
        &data_dir,
        &[
            "games",
            "instance",
            "content",
            "add",
            "valheim-test",
            &non_ts_item_id,
        ],
    );
    assert!(!out_place_unknown.status.success());
    let stderr = String::from_utf8_lossy(&out_place_unknown.stderr);
    assert!(
        stderr.contains("Cannot determine placement for content")
            || stderr.contains("Specify --into"),
        "stderr should mention placement failure: {stderr}"
    );

    // 5. Specifying --into works on non-Thunderstore item
    let out_place_manual = run_agora(
        &data_dir,
        &[
            "games",
            "instance",
            "content",
            "add",
            "valheim-test",
            &non_ts_item_id,
            "--into",
            "BepInEx/plugins",
        ],
    );
    assert!(
        out_place_manual.status.success(),
        "manual --into should succeed: {}",
        String::from_utf8_lossy(&out_place_manual.stderr)
    );

    // 6. Now add a Thunderstore package (with manifest.json)
    let ts_dir = tempdir();
    std::fs::write(
        ts_dir.path().join("manifest.json"),
        r#"{"name": "MyMod", "version_number": "1.0.0", "dependencies": ["Author-OtherMod-1.0.0"]}"#,
    )
    .unwrap();
    std::fs::write(ts_dir.path().join("MyMod.dll"), b"assembly").unwrap();
    let out_add_ts = run_agora(
        &data_dir,
        &[
            "games",
            "content",
            "add",
            ts_dir.path().to_str().unwrap(),
            "--name",
            "Author-MyMod-1.0.0.zip",
        ],
    );
    assert!(out_add_ts.status.success());
    let list_out2 = run_agora_json(&data_dir, &["games", "content", "list"]);
    let list_json2: serde_json::Value = serde_json::from_slice(&list_out2.stdout).unwrap();
    let ts_item_id = list_json2
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"].as_str().unwrap_or("").contains("MyMod"))
        .unwrap()["item_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 7. Add Thunderstore item without flags: places itself automatically and warns missing dependency!
    let out_place_ts = run_agora(
        &data_dir,
        &[
            "games",
            "instance",
            "content",
            "add",
            "valheim-test",
            &ts_item_id,
        ],
    );
    assert!(
        out_place_ts.status.success(),
        "thunderstore add should succeed: {}",
        String::from_utf8_lossy(&out_place_ts.stderr)
    );
    let stdout_ts = String::from_utf8_lossy(&out_place_ts.stdout);
    let stderr_ts = String::from_utf8_lossy(&out_place_ts.stderr);
    assert!(stdout_ts.contains("Thunderstore package Author-MyMod 1.0.0:"));
    assert!(stderr_ts.contains("Warning: MyMod needs Author-OtherMod; it is not in this instance"));
}

// ---------------------------------------------------------------------------
// The virtual file system cannot start: say so, and ask before stepping down
// ---------------------------------------------------------------------------

/// A pinned Skyrim SE instance in a disposable data root, built from a fake install whose
/// `SkyrimSE.exe` is a copy of `cmd.exe` (so a launch that runs ends at once). Nothing here reads
/// or writes the machine's real Skyrim, its Documents or its AppData.
struct VfsFixture {
    _tmp: TempDir,
    data_dir: PathBuf,
    user_data: PathBuf,
    missing_dll: PathBuf,
    instance_id: String,
}

fn vfs_fixture() -> VfsFixture {
    skyrim_fixture("1.6.1170.0", &[])
}

/// A Skyrim SE instance (no content) whose runtime is `version`. Each of `extra_files`, a path
/// relative to the install folder, is written into the install before the instance is created.
fn skyrim_fixture(version: &str, extra_files: &[&str]) -> VfsFixture {
    use agora_core::game_discovery::{DiscoveredInstall, InstallCapabilities};
    use agora_core::game_registry::{GameRegistry, IdentifiedInstall, PackageSource};
    use agora_game_api::{GameId, InstallId, InstallKind, RuntimeIdentity, StoreId};

    let tmp = tempdir();
    let data_dir = tmp.path().join("app_data");
    let install_dir = tmp.path().join("install");
    std::fs::create_dir_all(install_dir.join("Data")).unwrap();
    if cfg!(windows) {
        std::fs::copy(
            r"C:\Windows\System32\cmd.exe",
            install_dir.join("SkyrimSE.exe"),
        )
        .unwrap();
    } else {
        std::fs::write(install_dir.join("SkyrimSE.exe"), b"fake game binary").unwrap();
    }
    std::fs::write(install_dir.join("Data").join("Skyrim.esm"), b"ESM").unwrap();
    for extra in extra_files {
        let path = install_dir.join(extra);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"fake framework file").unwrap();
    }

    let mut builder = GameRegistry::builder();
    builder
        .add(
            PackageSource::Compiled {
                crate_name: "agora-game-creation".to_string(),
            },
            agora_game_creation::game_package(),
        )
        .expect("register the Creation Engine package");
    let ctx = agora_core::ctx::CoreContext::for_testing(data_dir.clone())
        .with_games(std::sync::Arc::new(builder.build()));
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();

    let store = StoreId::new("steam").unwrap();
    let runtime = RuntimeIdentity {
        game: GameId::new("skyrim-se").unwrap(),
        store: store.clone(),
        version: version.into(),
        build: None,
    };
    let volume =
        agora_core::game_discovery::volume::VolumeDetector::new().get_volume_info(&install_dir);
    let install = IdentifiedInstall {
        game: runtime.game.clone(),
        install_id: InstallId::new("steam:489830").unwrap(),
        discovered: DiscoveredInstall {
            store,
            product: "489830".into(),
            name: "Skyrim Special Edition".into(),
            kind: InstallKind::BaseGame,
            parent_product: None,
            location: install_dir.clone(),
            store_version: Some(runtime.version.clone()),
            store_build: None,
            executables: vec!["SkyrimSE.exe".into()],
            capabilities: InstallCapabilities {
                executables_readable: true,
                accepts_new_files: true,
                relocatable: true,
            },
            volume,
        },
        add_ons: vec![],
        runtime: agora_core::game_registry::RuntimeResolution::Identified {
            runtime,
            source: "executable".into(),
        },
    };
    let definition = ctx
        .games
        .game(&GameId::new("skyrim-se").unwrap())
        .expect("the Skyrim SE definition")
        .clone();
    let record = agora_core::game_instance::create(
        &ctx,
        &install,
        &definition,
        "Vfs Fallback",
        Some("vfs-fallback".to_string()),
        agora_core::game_base::BaseMode::Linked,
        &|_| {},
    )
    .expect("create the instance");

    let user_data = tmp.path().join("user_data");
    let missing_dll = tmp.path().join("no-such-dir").join("agora_vfs.dll");
    VfsFixture {
        _tmp: tmp,
        data_dir,
        user_data,
        missing_dll,
        instance_id: record.instance_id,
    }
}

/// Run the CLI against the fixture with no agora_vfs.dll to be found and per-user files redirected
/// into the fixture. Standard input is not a terminal, so nothing can be asked.
fn run_vfs_fixture(fixture: &VfsFixture, args: &[&str]) -> std::process::Output {
    let mut cmd = agora_command(&fixture.data_dir, args);
    cmd.env("AGORA_VFS_DLL", &fixture.missing_dll);
    cmd.env("AGORA_TEST_USER_DATA_ROOT", &fixture.user_data);
    run_command(cmd, args)
}

#[test]
fn a_launch_that_cannot_start_the_vfs_says_why_and_prints_both_ways_to_run_from_links() {
    let fixture = vfs_fixture();
    let id = fixture.instance_id.clone();
    let launch_cmd = format!("agora games instance launch {id} --deployment links");
    let keep_cmd = format!("agora games instance set-deployment {id} links");

    // Nobody chose a rung: it still asks (here: reports and exits) rather than stepping down.
    let out = run_vfs_fixture(&fixture, &["games", "instance", "launch", &id]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    println!(
        "--- stderr ---\n{stderr}--- stdout ---\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("the virtual file system could not start"),
        "{stderr}"
    );
    assert!(
        stderr.contains("AGORA_VFS_DLL") || stderr.contains("only available on Windows"),
        "{stderr}"
    );
    assert!(
        stderr.contains("Linked files cannot catch writes"),
        "{stderr}"
    );
    assert!(stderr.contains(&launch_cmd), "{stderr}");
    assert!(stderr.contains(&keep_cmd), "{stderr}");
    // No prompt without a terminal.
    assert!(!String::from_utf8_lossy(&out.stdout).contains("[y/N]"));
    assert!(!stderr.contains("[y/N]"));

    // The same when the user chose the virtual rung for the instance.
    let set = run_vfs_fixture(
        &fixture,
        &["games", "instance", "set-deployment", &id, "virtual"],
    );
    assert!(set.status.success());
    let out = run_vfs_fixture(&fixture, &["games", "instance", "launch", &id]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(&launch_cmd) && stderr.contains(&keep_cmd),
        "{stderr}"
    );

    // --json never prompts: the error object names the next rung and the two commands.
    let out = run_vfs_fixture(&fixture, &["--json", "games", "instance", "launch", &id]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    // Other warnings may precede the object on stderr.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let error: serde_json::Value = serde_json::from_str(&stderr[stderr.find('{').unwrap()..])
        .expect("the error is one JSON object");
    assert_eq!(error["status"], "error");
    assert_eq!(error["exitCode"], 1);
    assert_eq!(error["nextDeployment"], "links");
    assert_eq!(error["retry"], serde_json::json!([launch_cmd, keep_cmd]));
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("the virtual file system could not start"));
}

#[test]
fn the_fall_back_flag_runs_from_linked_files_without_asking() {
    let fixture = vfs_fixture();
    let id = fixture.instance_id.clone();

    let out = run_vfs_fixture(
        &fixture,
        &["games", "instance", "launch", &id, "--fall-back", "--wait"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    println!("--- stderr ---\n{stderr}--- stdout ---\n{stdout}");
    assert!(!stdout.contains("[y/N]") && !stderr.contains("[y/N]"));
    if !cfg!(windows) {
        // The fake game only runs on Windows; elsewhere the flag still must not ask.
        return;
    }
    assert!(
        stderr.contains("Notice: the virtual file system could not start")
            && stderr.contains("running from linked files instead"),
        "{stderr}"
    );
    assert!(stdout.contains("Program:"), "{stdout}");
    // Nothing is remembered: the next launch asks again.
    let after = run_vfs_fixture(&fixture, &["games", "instance", "launch", &id]);
    assert_eq!(after.status.code(), Some(1));

    // --json carries the same facts.
    let out = run_vfs_fixture(
        &fixture,
        &[
            "--json",
            "games",
            "instance",
            "launch",
            &id,
            "--fall-back",
            "--wait",
        ],
    );
    // stdout is the one JSON object and nothing else.
    let value = assert_json_stdout(&out);
    assert_eq!(value["status"], "exited");
    assert_eq!(value["deployment"], "links");
    assert!(value["notice"]
        .as_str()
        .unwrap()
        .contains("running from linked files instead"));
}

#[test]
fn a_json_launch_prints_only_one_json_object_on_stdout_with_or_without_wait() {
    if !cfg!(windows) {
        // The fake game only runs on Windows.
        return;
    }
    // Without --wait: the launch is reported at once.
    let fixture = vfs_fixture();
    let id = fixture.instance_id.clone();
    let out = run_vfs_fixture(
        &fixture,
        &["--json", "games", "instance", "launch", &id, "--fall-back"],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let value = assert_json_stdout(&out);
    assert_eq!(value["status"], "launched");
    assert_eq!(value["deployment"], "links");
    assert!(value["deploy_summary"]["status"].is_string(), "{value}");

    // With --wait: the session's per-user files and deployment come back in the object.
    let fixture = vfs_fixture();
    let id = fixture.instance_id.clone();
    let out = run_vfs_fixture(
        &fixture,
        &[
            "--json",
            "games",
            "instance",
            "launch",
            &id,
            "--fall-back",
            "--wait",
        ],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let value = assert_json_stdout(&out);
    assert_eq!(value["status"], "exited");
    assert!(value["user_files_restored"]["restored"].is_u64(), "{value}");
    assert!(value["deploy_summary"]["status"].is_string(), "{value}");
    assert!(value["session_ended_quickly"].is_boolean(), "{value}");
}

#[test]
fn launch_help_offers_fall_back() {
    let (_tmp, data_dir) = temp_data_dir();
    let out = run_agora(&data_dir, &["games", "instance", "launch", "--help"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("--fall-back"));
}

#[test]
fn games_instance_check_names_a_framework_built_for_another_version() {
    // SKSE for 1.6.1179 sits in a game that is 1.6.1170.0: the check refuses and names SKSE.
    let fixture = skyrim_fixture("1.6.1170.0", &["skse64_1_6_1179.dll"]);
    let id = fixture.instance_id.clone();

    let out = run_agora(&fixture.data_dir, &["games", "instance", "check", &id]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Skyrim Script Extender (SKSE)"), "{stdout}");
    assert!(stdout.contains("skse64_1_6_1170.dll"), "{stdout}");
    assert!(stdout.contains("skse64_1_6_1179.dll"), "{stdout}");

    let out_json = run_agora_json(&fixture.data_dir, &["games", "instance", "check", &id]);
    assert_eq!(out_json.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out_json.stdout).unwrap();
    assert_eq!(json["status"], "findings");
    assert_eq!(json["findings"][0]["rule_id"], "skse");
    assert_eq!(json["findings"][0]["problem"]["kind"], "wrong_version");
    assert_eq!(json["findings"][0]["found"][0], "skse64_1_6_1179.dll");

    // A launch makes the same check and refuses before it starts anything.
    let out_launch = run_agora(
        &fixture.data_dir,
        &["games", "instance", "launch", &id, "--deployment", "links"],
    );
    assert_eq!(out_launch.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out_launch.stderr);
    assert!(stderr.contains("Skyrim Script Extender (SKSE)"), "{stderr}");
    assert!(stderr.contains("--launch-anyway"), "{stderr}");
}

#[test]
fn games_instance_check_passes_a_runtime_with_its_own_frameworks() {
    let fixture = skyrim_fixture("1.6.1170.0", &["skse64_1_6_1170.dll"]);
    let id = fixture.instance_id.clone();

    let out = run_agora(&fixture.data_dir, &["games", "instance", "check", &id]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("No framework problems"));

    let out_json = run_agora_json(&fixture.data_dir, &["games", "instance", "check", &id]);
    assert_eq!(out_json.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out_json.stdout).unwrap();
    assert_eq!(json["findings"].as_array().map(Vec::len), Some(0));
    // The fixture's Skyrim.esm is not a real plugin, so the load order warns that its header cannot
    // be read. That is a warning: it never fails the check.
    assert_eq!(json["status"], "warnings");
    assert_eq!(json["load_order_findings"][0]["kind"], "unreadable_header");
    assert_eq!(json["load_order_findings"][0]["plugin"], "Skyrim.esm");
}

#[test]
fn games_instance_check_lists_a_rule_it_cannot_check_but_exits_zero() {
    // A three-part version cannot fill the Address Library rule's {4}: a definition-side
    // finding, listed as a warning. It does not fail the check.
    let fixture = skyrim_fixture("1.6.1170", &[]);
    let id = fixture.instance_id.clone();

    let out = run_agora(&fixture.data_dir, &["games", "instance", "check", &id]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cannot be checked"), "{stdout}");
    assert!(stdout.contains("component 4"), "{stdout}");

    let out_json = run_agora_json(&fixture.data_dir, &["games", "instance", "check", &id]);
    assert_eq!(out_json.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out_json.stdout).unwrap();
    assert_eq!(json["status"], "warnings");
    assert_eq!(json["findings"][0]["problem"]["kind"], "cannot_check");
}

#[test]
fn games_instance_ini_and_saves_round_trip_through_the_cli() {
    // The game's files live in a temporary user-data root, never the real Documents folder: the
    // child process is pointed at it.
    let fixture = skyrim_fixture("1.6.1170.0", &[]);
    let id = fixture.instance_id.clone();
    let real_ini = fixture
        .user_data
        .join("documents")
        .join("My Games")
        .join("Skyrim Special Edition")
        .join("Skyrim.ini");
    std::fs::create_dir_all(real_ini.parent().unwrap()).unwrap();
    let original = "[General]\r\nsLanguage=ENGLISH\r\n[Display]\r\nfGamma=1.0\r\n";
    std::fs::write(&real_ini, original).unwrap();

    let run = |args: &[&str]| {
        let mut cmd = agora_command(&fixture.data_dir, args);
        cmd.env("AGORA_TEST_USER_DATA_ROOT", &fixture.user_data);
        run_command(cmd, args)
    };
    let text = |out: &std::process::Output| String::from_utf8_lossy(&out.stdout).into_owned();

    let listed = run(&["games", "instance", "ini", &id]);
    assert_eq!(listed.status.code(), Some(0));
    assert!(
        text(&listed).contains("user/Skyrim.ini"),
        "{}",
        text(&listed)
    );
    assert!(text(&listed).contains("copy: no"), "{}", text(&listed));

    let missing = run(&[
        "games",
        "instance",
        "ini",
        &id,
        "user/Skyrim.ini",
        "General",
        "SLocalSavePath",
    ]);
    assert_eq!(
        missing.status.code(),
        Some(1),
        "a key that is not set exits 1"
    );

    let set = run(&[
        "games",
        "instance",
        "ini",
        &id,
        "user/Skyrim.ini",
        "Display",
        "fGamma",
        "2.2",
    ]);
    assert_eq!(set.status.code(), Some(0));
    let got = run(&[
        "games",
        "instance",
        "ini",
        &id,
        "user/Skyrim.ini",
        "display",
        "fgamma",
    ]);
    assert_eq!(text(&got).trim(), "2.2");

    let saves = run(&["games", "instance", "saves", &id]);
    assert_eq!(saves.status.code(), Some(0));
    assert!(text(&saves).contains("shared"), "{}", text(&saves));

    let own = run(&["games", "instance", "saves", &id, "own"]);
    assert_eq!(
        own.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&own.stderr)
    );
    assert!(text(&own).contains("stay where they are"), "{}", text(&own));
    let own_value = run(&[
        "games",
        "instance",
        "ini",
        &id,
        "user/Skyrim.ini",
        "General",
        "SLocalSavePath",
    ]);
    assert_eq!(own_value.status.code(), Some(0));
    assert!(
        text(&own_value).trim().starts_with(r"Saves\Agora\"),
        "{}",
        text(&own_value)
    );

    let json_shared = run(&["--json", "games", "instance", "saves", &id, "shared"]);
    assert_eq!(json_shared.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&json_shared.stdout).unwrap();
    assert_eq!(json["choice"], "shared");
    assert_eq!(json["previous"], "own");
    assert_eq!(json["setting"]["action"], "removed");

    let gone = run(&[
        "games",
        "instance",
        "ini",
        &id,
        "user/Skyrim.ini",
        "General",
        "SLocalSavePath",
    ]);
    assert_eq!(
        gone.status.code(),
        Some(1),
        "shared removed the key it added"
    );

    let bad = run(&["games", "instance", "saves", &id, "fastest"]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("not a save choice"));

    let unset = run(&[
        "games",
        "instance",
        "ini",
        &id,
        "user/Skyrim.ini",
        "Display",
        "fGamma",
        "--unset",
    ]);
    assert_eq!(unset.status.code(), Some(0));

    assert_eq!(
        std::fs::read_to_string(&real_ini).unwrap(),
        original,
        "the game's own file was never edited"
    );
}

#[test]
fn the_shipped_skyrim_package_gives_each_store_its_own_save_location() {
    use agora_game_api::StoreId;

    let package = agora_game_creation::game_package();
    let skyrim = package
        .definition()
        .games
        .iter()
        .find(|g| g.id.as_str() == "skyrim-se")
        .expect("the package defines skyrim-se");
    let steam = skyrim
        .save_location
        .iter()
        .find(|r| r.applies_to_store(&StoreId::steam()))
        .expect("steam has a save location");
    let gog = skyrim
        .save_location
        .iter()
        .find(|r| r.applies_to_store(&StoreId::gog()))
        .expect("gog has a save location");
    assert_eq!(steam.ini.as_str(), "user/Skyrim.ini");
    assert_eq!(steam.section, "General");
    assert_eq!(steam.key, "SLocalSavePath");
    assert_eq!(steam.own_value, r"Saves\Agora\{instance}\");
    assert_ne!(steam.shared_dir, gog.shared_dir);
    // GOG's folder is named as its own user files name it.
    assert!(
        serde_json::to_string(&gog.shared_dir)
            .unwrap()
            .contains("Skyrim Special Edition GOG/Saves"),
        "{:?}",
        gog.shared_dir
    );
}

// ---------------------------------------------------------------------------
// Catalog entries for other games (MASTER_SPEC §26.8)
// ---------------------------------------------------------------------------

/// A registry database as the compiler writes it: schema 9, with the other
/// games' entries in `game_catalog_items`. One Skyrim SE entry whose
/// compatibility names stores, versions, SKSE and an asset.
///
/// Written outside the data directory and handed to the debug build through
/// `AGORA_DEV_REGISTRY_DB`, because a cached registry without an Ed25519
/// signature is quarantined and no test can sign one.
fn write_catalog_fixture(db_path: &Path) {
    let sha = "a".repeat(64);
    let skyrim_compat = r#"[{"stores":["steam","gog"],"game_versions":["1.6.1170.*"],"requires":[{"framework":"skse","min_version":"2.2.6"}],"asset":"CrashLogger-*.7z"}]"#;
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE schema_version (version INTEGER PRIMARY KEY);
         INSERT INTO schema_version (version) VALUES (9);
         CREATE TABLE game_catalog_items (
            id TEXT PRIMARY KEY, game TEXT NOT NULL, name TEXT NOT NULL,
            author TEXT, content_type TEXT NOT NULL, download_strategy TEXT NOT NULL,
            source_identifier TEXT NOT NULL, sha256 TEXT NOT NULL,
            download_sources_json TEXT NOT NULL DEFAULT '[]',
            game_compatibility_json TEXT NOT NULL, description TEXT,
            license_id TEXT, page_url TEXT, icon_url TEXT,
            status TEXT NOT NULL DEFAULT 'active', date_added TEXT
         );
         INSERT INTO game_catalog_items (id, game, name, content_type, download_strategy,
            source_identifier, sha256, download_sources_json, game_compatibility_json, license_id)
         VALUES ('crash-logger', 'skyrim-se', 'CrashLogger', 'mod', 'github_release',
            'example-author/crash-logger', '{sha}',
            '[{{\"strategy\":\"github_release\",\"identifier\":\"example-author/crash-logger\"}}]',
            '{}', 'MIT');",
        skyrim_compat.replace('\'', "''")
    ))
    .unwrap();
}

/// Run agora against the fixture registry, with `--json` when asked.
fn run_agora_with_catalog(
    data_dir: &Path,
    catalog_db: &Path,
    json: bool,
    args: &[&str],
) -> std::process::Output {
    let mut full_args: Vec<&str> = Vec::new();
    if json {
        full_args.push("--json");
    }
    full_args.extend_from_slice(args);
    let mut cmd = agora_command(data_dir, &full_args);
    cmd.env("AGORA_DEV_REGISTRY_DB", catalog_db);
    run_command(cmd, &full_args)
}

#[test]
fn games_catalog_list_shows_the_other_game_entries_human_and_json() {
    let (_tmp, data_dir) = temp_data_dir();
    let catalog = tempfile::tempdir().unwrap();
    let catalog_db = catalog.path().join("registry.db");
    write_catalog_fixture(&catalog_db);

    let output = run_agora_with_catalog(
        &data_dir,
        &catalog_db,
        false,
        &["games", "catalog", "list", "skyrim-se"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("crash-logger"), "{stdout}");
    assert!(stdout.contains("steam/gog"), "{stdout}");
    assert!(
        !stdout.contains("sodium"),
        "Minecraft entries are not listed for Skyrim:\n{stdout}"
    );

    let json_output = run_agora_with_catalog(
        &data_dir,
        &catalog_db,
        true,
        &["games", "catalog", "list", "skyrim-se"],
    );
    assert!(
        json_output.status.success(),
        "{}",
        String::from_utf8_lossy(&json_output.stderr)
    );
    let entries: serde_json::Value = serde_json::from_slice(&json_output.stdout).unwrap();
    let entries = entries.as_array().expect("a JSON array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], "crash-logger");
    assert_eq!(
        entries[0]["game_compatibility"][0]["asset"],
        "CrashLogger-*.7z"
    );
    assert_eq!(
        entries[0]["game_compatibility"][0]["requires"][0]["min_version"],
        "2.2.6"
    );
}

#[test]
fn games_catalog_list_refuses_minecraft_and_unknown_games() {
    let (_tmp, data_dir) = temp_data_dir();
    let catalog = tempfile::tempdir().unwrap();
    let catalog_db = catalog.path().join("registry.db");
    write_catalog_fixture(&catalog_db);
    for game in ["minecraft", "no-such-game"] {
        let output = run_agora_with_catalog(
            &data_dir,
            &catalog_db,
            false,
            &["games", "catalog", "list", game],
        );
        assert!(!output.status.success(), "{game} should be refused");
    }
}

#[test]
fn games_catalog_install_refuses_an_unknown_entry_or_instance_before_any_download() {
    let (_tmp, data_dir) = temp_data_dir();
    let catalog = tempfile::tempdir().unwrap();
    let catalog_db = catalog.path().join("registry.db");
    write_catalog_fixture(&catalog_db);

    let unknown_entry = run_agora_with_catalog(
        &data_dir,
        &catalog_db,
        false,
        &[
            "games",
            "catalog",
            "install",
            "no-such-instance",
            "no-such-entry",
            "--dry-run",
        ],
    );
    assert!(!unknown_entry.status.success());
    let stderr = String::from_utf8_lossy(&unknown_entry.stderr);
    assert!(
        stderr.contains("No catalog entry 'no-such-entry'"),
        "{stderr}"
    );

    let unknown_instance = run_agora_with_catalog(
        &data_dir,
        &catalog_db,
        false,
        &[
            "games",
            "catalog",
            "install",
            "no-such-instance",
            "crash-logger",
            "--dry-run",
        ],
    );
    assert!(!unknown_instance.status.success());
    let stderr = String::from_utf8_lossy(&unknown_instance.stderr);
    assert!(stderr.contains("not found"), "{stderr}");
    assert!(
        !String::from_utf8_lossy(&unknown_instance.stdout).contains("Downloading"),
        "nothing is announced for a download that cannot start"
    );
}

// ---------------------------------------------------------------------------
// Updates, pins, changelogs and pack updates
// ---------------------------------------------------------------------------

/// The manifest entry for one installed file, as the instance records it.
fn manifest_entry(data_dir: &Path, instance: &str, file: &str) -> serde_json::Value {
    let path = data_dir
        .join("instances")
        .join(instance)
        .join("instance_manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    // A manifest Agora has written keeps the Minecraft fields under "minecraft"; one a test wrote
    // by hand keeps them at the top.
    let mods = if manifest["minecraft"]["mods"].is_array() {
        &manifest["minecraft"]["mods"]
    } else {
        &manifest["mods"]
    };
    mods.as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["filename"] == file)
        .cloned()
        .unwrap_or_else(|| panic!("{file} is not in the manifest"))
}

/// A minimal .mrpack for a 1.21 Fabric instance: one config override, no jars.
fn write_test_mrpack(path: &Path, config_body: &str) {
    use zip::write::SimpleFileOptions;
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    let index = serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "2.0.0",
        "name": "Updates Test Pack",
        "dependencies": { "minecraft": "1.21", "fabric-loader": "0.16.0" },
        "files": [],
    });
    zip.start_file("modrinth.index.json", options).unwrap();
    zip.write_all(index.to_string().as_bytes()).unwrap();
    zip.start_file("overrides/config/updates-test.toml", options)
        .unwrap();
    zip.write_all(config_body.as_bytes()).unwrap();
    zip.finish().unwrap();
}

#[test]
fn instance_updates_reports_nothing_for_hand_installed_mods() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "upd-none", &["manual.jar"]);

    let output = run_agora(&data_dir, &["instance", "updates", "upd-none"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("No updates are available for 'upd-none'"),
        "{stdout}"
    );
}

#[test]
fn instance_updates_json_is_an_empty_list_when_nothing_is_newer() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "upd-json", &["manual.jar"]);

    let output = run_agora_json(&data_dir, &["instance", "updates", "upd-json"]);
    assert!(output.status.success());
    assert_eq!(assert_json_stdout(&output), serde_json::json!([]));
}

#[test]
fn instance_updates_apply_with_nothing_to_apply_changes_nothing() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "upd-apply-none", &["manual.jar"]);

    let output = run_agora(
        &data_dir,
        &["instance", "updates", "upd-apply-none", "--apply"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("No updates to apply"), "{stdout}");
}

#[test]
fn instance_updates_refuses_an_unknown_instance() {
    let (_tmp, data_dir) = temp_data_dir();

    let output = run_agora(&data_dir, &["instance", "updates", "no-such-instance"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("'no-such-instance' not found"),
        "stderr should name the instance:\n{stderr}"
    );
}

#[test]
fn instance_updates_yes_without_apply_is_refused_by_the_parser() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "upd-yes", &["manual.jar"]);

    let output = run_agora(&data_dir, &["instance", "updates", "upd-yes", "--yes"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--apply"), "{stderr}");
}

#[test]
fn mod_pin_and_unpin_record_the_choice_in_the_manifest() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "pin-round", &["sodium.jar"]);

    let pin = run_agora(&data_dir, &["mod", "pin", "pin-round", "sodium.jar"]);
    assert!(
        pin.status.success(),
        "{}",
        String::from_utf8_lossy(&pin.stderr)
    );
    assert_eq!(
        manifest_entry(&data_dir, "pin-round", "sodium.jar")["update_pinned"],
        true
    );

    let unpin = run_agora(&data_dir, &["mod", "unpin", "pin-round", "sodium.jar"]);
    assert!(unpin.status.success());
    assert_eq!(
        manifest_entry(&data_dir, "pin-round", "sodium.jar")["update_pinned"],
        false
    );
}

#[test]
fn mod_pin_unknown_file_fails_and_changes_nothing() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "pin-ghost", &["sodium.jar"]);

    let output = run_agora(&data_dir, &["mod", "pin", "pin-ghost", "ghost.jar"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ghost.jar") && stderr.contains("not installed"),
        "stderr should name the missing file:\n{stderr}"
    );
    // An unpinned entry leaves `update_pinned` out of the manifest, so "not pinned" is the check.
    assert!(manifest_entry(&data_dir, "pin-ghost", "sodium.jar")["update_pinned"] != true);
}

#[test]
fn mod_changelog_has_nothing_to_show_for_a_hand_installed_mod() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "log-none", &["manual.jar"]);

    let output = run_agora(&data_dir, &["mod", "changelog", "log-none", "manual.jar"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("has no newer version"), "{stdout}");
}

#[test]
fn mod_changelog_unknown_file_fails() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "log-ghost", &["sodium.jar"]);

    let output = run_agora(&data_dir, &["mod", "changelog", "log-ghost", "ghost.jar"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not installed"), "{stderr}");
}

#[test]
fn pack_update_preview_lists_the_change_and_writes_nothing() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "pack-preview", &["sodium.jar"]);
    let mrpack = data_dir.join("updates-test-pack.mrpack");
    write_test_mrpack(&mrpack, "pack = true\n");
    let mrpack_arg = mrpack.to_string_lossy().to_string();

    let output = run_agora(
        &data_dir,
        &[
            "pack",
            "update-preview",
            "pack-preview",
            mrpack_arg.as_str(),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Updates Test Pack"), "{stdout}");
    assert!(stdout.contains("add: 1"), "{stdout}");
    assert!(
        !data_dir
            .join("instances/pack-preview/config/updates-test.toml")
            .exists(),
        "a preview must not write the pack's file"
    );
}

#[test]
fn pack_update_with_yes_applies_the_new_config() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "pack-apply", &["sodium.jar"]);
    let mrpack = data_dir.join("updates-test-pack.mrpack");
    write_test_mrpack(&mrpack, "pack = true\n");
    let mrpack_arg = mrpack.to_string_lossy().to_string();

    let output = run_agora(
        &data_dir,
        &["pack", "update", "pack-apply", mrpack_arg.as_str(), "--yes"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Updated 'pack-apply'"), "{stdout}");
    assert_eq!(
        std::fs::read_to_string(data_dir.join("instances/pack-apply/config/updates-test.toml"))
            .unwrap(),
        "pack = true\n"
    );
}

#[test]
fn pack_update_without_yes_changes_nothing() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "pack-noyes", &["sodium.jar"]);
    let mrpack = data_dir.join("updates-test-pack.mrpack");
    write_test_mrpack(&mrpack, "pack = true\n");
    let mrpack_arg = mrpack.to_string_lossy().to_string();

    let output = run_agora(
        &data_dir,
        &["pack", "update", "pack-noyes", mrpack_arg.as_str()],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Nothing was changed"), "{stderr}");
    assert!(!data_dir
        .join("instances/pack-noyes/config/updates-test.toml")
        .exists());
}

#[test]
fn pack_update_refuses_an_unanswered_conflict_and_keeps_the_users_file() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "pack-conflict", &["sodium.jar"]);
    let config = data_dir.join("instances/pack-conflict/config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("updates-test.toml"), "user edit\n").unwrap();
    let mrpack = data_dir.join("updates-test-pack.mrpack");
    write_test_mrpack(&mrpack, "pack = true\n");
    let mrpack_arg = mrpack.to_string_lossy().to_string();

    let output = run_agora(
        &data_dir,
        &[
            "pack",
            "update",
            "pack-conflict",
            mrpack_arg.as_str(),
            "--yes",
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("need an answer"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(config.join("updates-test.toml")).unwrap(),
        "user edit\n"
    );
}

#[test]
fn pack_update_refuses_an_empty_or_repeated_answer() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "pack-hostile", &["sodium.jar"]);
    let mrpack = data_dir.join("updates-test-pack.mrpack");
    write_test_mrpack(&mrpack, "pack = true\n");
    let mrpack_arg = mrpack.to_string_lossy().to_string();

    let empty = run_agora(
        &data_dir,
        &[
            "pack",
            "update",
            "pack-hostile",
            mrpack_arg.as_str(),
            "--keep-ours",
            "  ",
            "--yes",
        ],
    );
    assert!(!empty.status.success());
    assert!(
        String::from_utf8_lossy(&empty.stderr).contains("cannot be empty"),
        "a blank key must be refused"
    );

    let repeated = run_agora(
        &data_dir,
        &[
            "pack",
            "update",
            "pack-hostile",
            mrpack_arg.as_str(),
            "--keep-ours",
            "config/a.toml",
            "--take-theirs",
            "config/a.toml",
            "--yes",
        ],
    );
    assert!(!repeated.status.success());
    assert!(
        String::from_utf8_lossy(&repeated.stderr).contains("answered more than once"),
        "one key answered both ways must be refused"
    );
}

#[test]
fn pack_update_refuses_an_answer_for_a_key_that_is_not_a_conflict() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "pack-typo", &["sodium.jar"]);
    let mrpack = data_dir.join("updates-test-pack.mrpack");
    write_test_mrpack(&mrpack, "pack = true\n");
    let mrpack_arg = mrpack.to_string_lossy().to_string();

    let output = run_agora(
        &data_dir,
        &[
            "pack",
            "update",
            "pack-typo",
            mrpack_arg.as_str(),
            "--take-theirs",
            "config/nonsense.toml",
            "--yes",
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("is not a conflict"), "{stderr}");
    assert!(!data_dir
        .join("instances/pack-typo/config/updates-test.toml")
        .exists());
}

// ---------------------------------------------------------------------------
// Guided mod bisect (`agora bisect`)
// ---------------------------------------------------------------------------

const BISECT_MODS: &[&str] = &["alpha.jar", "bravo.jar", "charlie.jar", "delta.jar"];

/// `(enabled, disabled)` mod file names in an instance's `mods` folder, with
/// the `.disabled` suffix removed from the second list. Both sorted.
fn bisect_mod_state(data_dir: &std::path::Path, id: &str) -> (Vec<String>, Vec<String>) {
    let mods = data_dir.join("instances").join(id).join("mods");
    let mut on = Vec::new();
    let mut off = Vec::new();
    for entry in std::fs::read_dir(&mods).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        match name.strip_suffix(".disabled") {
            Some(base) => off.push(base.to_string()),
            None => on.push(name),
        }
    }
    on.sort();
    off.sort();
    (on, off)
}

fn bisect_session_file(data_dir: &std::path::Path, id: &str) -> std::path::PathBuf {
    data_dir
        .join("instances")
        .join(id)
        .join(".agora_bisect.json")
}

fn sorted_strings(value: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = value
        .as_array()
        .expect("expected a JSON array")
        .iter()
        .map(|v| v.as_str().expect("expected a string").to_string())
        .collect();
    out.sort();
    out
}

fn bisect_instance(data_dir: &std::path::Path, id: &str) {
    create_loader_instance(data_dir, id, BISECT_MODS);
}

#[test]
fn bisect_start_plans_a_session_over_every_enabled_mod_and_moves_nothing() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-start");

    let output = run_agora_json(
        &data_dir,
        &[
            "bisect",
            "start",
            "bisect-start",
            "--suspect",
            "charlie.jar",
        ],
    );
    assert!(
        output.status.success(),
        "bisect start should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = assert_json_stdout(&output);
    let suspects = value["session"]["suspects"].as_array().unwrap();
    assert_eq!(suspects.len(), 4);
    assert_eq!(
        suspects[0], "charlie.jar",
        "a crash-named mod is tested first"
    );
    assert_eq!(value["trial"]["status"]["type"], "awaiting_trial");
    assert!(bisect_session_file(&data_dir, "bisect-start").is_file());
    let (on, off) = bisect_mod_state(&data_dir, "bisect-start");
    assert_eq!(on.len(), 4);
    assert!(off.is_empty(), "starting must not turn any mod off");
}

#[test]
fn bisect_start_refuses_a_second_bisect_and_keeps_the_first() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-twice");
    let first = run_agora(&data_dir, &["bisect", "start", "bisect-twice"]);
    assert!(first.status.success());
    let session = std::fs::read_to_string(bisect_session_file(&data_dir, "bisect-twice")).unwrap();

    let second = run_agora(&data_dir, &["bisect", "start", "bisect-twice"]);
    assert!(!second.status.success(), "a second start must refuse");
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("already in progress"),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(bisect_session_file(&data_dir, "bisect-twice")).unwrap(),
        session,
        "a refused start must leave the session as it was"
    );
}

#[test]
fn bisect_start_needs_at_least_two_enabled_mods() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "bisect-solo", &["solo.jar"]);
    let output = run_agora(&data_dir, &["bisect", "start", "bisect-solo"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("at least two"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!bisect_session_file(&data_dir, "bisect-solo").exists());
}

#[test]
fn bisect_status_with_no_bisect_says_so_and_reports_null_in_json() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-idle");

    let human = run_agora(&data_dir, &["bisect", "status", "bisect-idle"]);
    assert!(human.status.success());
    assert!(String::from_utf8_lossy(&human.stdout).contains("No bisect is running"));

    let json = run_agora_json(&data_dir, &["bisect", "status", "bisect-idle"]);
    assert!(json.status.success());
    let value = assert_json_stdout(&json);
    assert!(value["session"].is_null());
    assert!(value["trial"].is_null());
}

#[test]
fn bisect_status_refuses_an_instance_that_does_not_exist() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["bisect", "status", "no-such-instance"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not found"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn bisect_apply_turns_off_exactly_the_mods_the_trial_holds_out() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-apply");
    run_agora(&data_dir, &["bisect", "start", "bisect-apply"]);

    let output = run_agora_json(&data_dir, &["bisect", "apply", "bisect-apply"]);
    assert!(
        output.status.success(),
        "bisect apply should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = assert_json_stdout(&output);
    let held_out = sorted_strings(&value["trial"]["disable"]);
    assert_eq!(held_out.len(), 2);
    let (on, off) = bisect_mod_state(&data_dir, "bisect-apply");
    assert_eq!(off, held_out, "exactly the held-out mods are off");
    assert_eq!(on.len(), 2);
}

#[test]
fn bisect_apply_without_a_session_is_refused_and_moves_nothing() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-apply-idle");
    let output = run_agora(&data_dir, &["bisect", "apply", "bisect-apply-idle"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("No bisect is in progress"));
    let (on, off) = bisect_mod_state(&data_dir, "bisect-apply-idle");
    assert_eq!(on.len(), 4);
    assert!(off.is_empty());
}

#[test]
fn bisect_good_keeps_the_held_out_half_as_the_suspects() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-good");
    run_agora(&data_dir, &["bisect", "start", "bisect-good"]);
    let applied = assert_json_stdout(&run_agora_json(
        &data_dir,
        &["bisect", "apply", "bisect-good"],
    ));
    let held_out = sorted_strings(&applied["trial"]["disable"]);

    let output = run_agora_json(&data_dir, &["bisect", "good", "bisect-good"]);
    assert!(
        output.status.success(),
        "bisect good should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = assert_json_stdout(&output);
    assert_eq!(sorted_strings(&value["session"]["suspects"]), held_out);
    assert_eq!(value["session"]["history"][0]["outcome"], "clean");
}

#[test]
fn bisect_good_without_a_session_is_refused() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-good-idle");
    let output = run_agora(&data_dir, &["bisect", "good", "bisect-good-idle"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("No bisect is in progress"));
}

#[test]
fn bisect_bad_keeps_the_mods_that_were_left_on_as_the_suspects() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-bad");
    run_agora(&data_dir, &["bisect", "start", "bisect-bad"]);
    run_agora(&data_dir, &["bisect", "apply", "bisect-bad"]);
    let (left_on, _) = bisect_mod_state(&data_dir, "bisect-bad");

    let output = run_agora_json(&data_dir, &["bisect", "bad", "bisect-bad"]);
    assert!(
        output.status.success(),
        "bisect bad should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = assert_json_stdout(&output);
    assert_eq!(sorted_strings(&value["session"]["suspects"]), left_on);
    assert_eq!(value["session"]["history"][0]["outcome"], "reproduced");
}

#[test]
fn bisect_bad_without_a_session_is_refused() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-bad-idle");
    let output = run_agora(&data_dir, &["bisect", "bad", "bisect-bad-idle"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("No bisect is in progress"));
}

#[test]
fn bisect_back_takes_the_other_half_on_the_next_trial() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-back");
    let started = assert_json_stdout(&run_agora_json(
        &data_dir,
        &["bisect", "start", "bisect-back"],
    ));
    let first_held_out = sorted_strings(&started["trial"]["disable"]);
    run_agora(&data_dir, &["bisect", "bad", "bisect-back"]);

    let output = run_agora_json(&data_dir, &["bisect", "back", "bisect-back"]);
    assert!(
        output.status.success(),
        "bisect back should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = assert_json_stdout(&output);
    assert_eq!(value["session"]["suspects"].as_array().unwrap().len(), 4);
    assert_ne!(sorted_strings(&value["trial"]["disable"]), first_held_out);
}

#[test]
fn bisect_back_with_nothing_to_undo_is_refused() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-back-fresh");
    run_agora(&data_dir, &["bisect", "start", "bisect-back-fresh"]);
    let output = run_agora(&data_dir, &["bisect", "back", "bisect-back-fresh"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nothing to step back"));
}

#[test]
fn bisect_cancel_puts_every_mod_back_and_ends_the_session() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-cancel");
    run_agora(&data_dir, &["bisect", "start", "bisect-cancel"]);
    run_agora(&data_dir, &["bisect", "apply", "bisect-cancel"]);
    let (_, off_before) = bisect_mod_state(&data_dir, "bisect-cancel");
    assert_eq!(off_before.len(), 2, "the trial turned two mods off");

    let output = run_agora_json(&data_dir, &["bisect", "cancel", "bisect-cancel"]);
    assert!(
        output.status.success(),
        "bisect cancel should succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = assert_json_stdout(&output);
    assert_eq!(value["status"], "cancelled");
    let (on, off) = bisect_mod_state(&data_dir, "bisect-cancel");
    assert_eq!(on, BISECT_MODS.to_vec(), "every mod is back on");
    assert!(off.is_empty());
    assert!(!bisect_session_file(&data_dir, "bisect-cancel").exists());
}

#[test]
fn bisect_cancel_with_nothing_running_is_a_quiet_no_op() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-cancel-idle");
    let output = run_agora_json(&data_dir, &["bisect", "cancel", "bisect-cancel-idle"]);
    assert!(output.status.success());
    assert_eq!(assert_json_stdout(&output)["status"], "not_running");
    let (on, _) = bisect_mod_state(&data_dir, "bisect-cancel-idle");
    assert_eq!(on.len(), 4);
}

#[test]
fn bisect_run_needs_a_terminal_and_changes_nothing() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-run");
    // Tests run with no terminal on stdin, so the interactive loop must refuse
    // before it starts a session or turns any mod off.
    let output = run_agora(&data_dir, &["bisect", "run", "bisect-run"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("interactive terminal"));
    assert!(!bisect_session_file(&data_dir, "bisect-run").exists());
    let (on, off) = bisect_mod_state(&data_dir, "bisect-run");
    assert_eq!(on.len(), 4);
    assert!(off.is_empty());
}

#[test]
fn bisect_run_refuses_json_because_it_prompts() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-run-json");
    let output = run_agora_json(&data_dir, &["bisect", "run", "bisect-run-json"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not print JSON"));
    assert!(!bisect_session_file(&data_dir, "bisect-run-json").exists());
}

#[test]
fn bisect_start_refuses_a_locked_instance_and_writes_nothing() {
    let (_tmp, data_dir) = temp_data_dir();
    bisect_instance(&data_dir, "bisect-locked");
    let manifest_path = data_dir
        .join("instances")
        .join("bisect-locked")
        .join("instance_manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    manifest["is_locked"] = serde_json::Value::Bool(true);
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let output = run_agora(&data_dir, &["bisect", "start", "bisect-locked"]);
    assert!(
        !output.status.success(),
        "a locked instance must refuse a bisect"
    );
    assert!(!bisect_session_file(&data_dir, "bisect-locked").exists());
    let (on, off) = bisect_mod_state(&data_dir, "bisect-locked");
    assert_eq!(on.len(), 4);
    assert!(off.is_empty());
}

// ---------------------------------------------------------------------------
// Governance, registry review queries, Technic, provider packs, plugin settings
// ---------------------------------------------------------------------------

/// The seeded registry lives beside the data root, not as `registry.db`: a
/// plain `registry.db` there is quarantined at startup for lacking a signature.
/// It is read only through `AGORA_DEV_REGISTRY_DB`, which debug builds honour.
fn dev_registry_path(data_dir: &Path) -> PathBuf {
    data_dir.join("dev-registry.db")
}

/// Run the CLI with the seeded dev registry active (debug builds only).
fn run_seeded(data_dir: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = agora_command(data_dir, args);
    cmd.env("AGORA_DEV_REGISTRY_DB", dev_registry_path(data_dir));
    run_command(cmd, args)
}

/// A schema-7 registry with one item under review, a governance summary and
/// events for `test-mod`, a curator review, an audit log, and a triage
/// resolution. Only the tables the read paths query are created.
fn seed_governance_registry(data_dir: &Path) {
    let conn = rusqlite::Connection::open(dev_registry_path(data_dir)).unwrap();
    conn.execute_batch(
        "CREATE TABLE schema_version (version INTEGER PRIMARY KEY);
         INSERT INTO schema_version (version) VALUES (7);

         CREATE TABLE registry_items (
             id TEXT PRIMARY KEY, name TEXT, content_type TEXT, icon_url TEXT,
             net_score INTEGER DEFAULT 0, status TEXT DEFAULT 'active'
         );
         INSERT INTO registry_items (id, name, content_type, net_score, status)
         VALUES ('test-mod', 'Test Mod', 'mod', 5, 'active'),
                ('flagged-mod', 'Flagged Mod', 'mod', -3, 'under_review');

         CREATE TABLE governance_summary (
             item_id TEXT PRIMARY KEY, vote_issue_number INTEGER, vote_issue_url TEXT,
             raw_upvotes INTEGER NOT NULL DEFAULT 0, raw_downvotes INTEGER NOT NULL DEFAULT 0,
             counted_upvotes INTEGER NOT NULL DEFAULT 0, counted_downvotes INTEGER NOT NULL DEFAULT 0,
             quarantined_upvotes INTEGER NOT NULL DEFAULT 0, quarantined_downvotes INTEGER NOT NULL DEFAULT 0,
             conflicted_users INTEGER NOT NULL DEFAULT 0, status_reason TEXT,
             compiled_at TEXT NOT NULL
         );
         INSERT INTO governance_summary VALUES
             ('test-mod', 42, 'https://github.com/example/repo/issues/42',
              10, 3, 8, 2, 2, 1, 0, NULL, '2026-10-01T00:00:00Z');

         CREATE TABLE governance_events (
             event_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, event_type TEXT NOT NULL,
             status TEXT NOT NULL, detected_at TEXT NOT NULL,
             affected_reactions INTEGER NOT NULL DEFAULT 0, details_json TEXT
         );
         INSERT INTO governance_events VALUES
             ('evt-001', 'test-mod', 'vote_quarantine', 'active', '2026-10-02T00:00:00Z', 3, NULL),
             ('evt-002', 'test-mod', 'status_change', 'under_review', '2026-10-03T00:00:00Z', 0, NULL);

         CREATE TABLE curator_reviews (item_id TEXT PRIMARY KEY, top_reviews_json TEXT);
         INSERT INTO curator_reviews VALUES
             ('flagged-mod', '[{\"author\":\"curator\",\"text\":\"Crashes on load\",\"issue_number\":77,\"created_at\":\"2026-10-01\"}]');

         CREATE TABLE audit_log (
             id INTEGER PRIMARY KEY, timestamp TEXT NOT NULL, action TEXT NOT NULL, details TEXT
         );
         INSERT INTO audit_log VALUES (1, '2026-10-04T00:00:00Z', 'triage_keep', 'flagged-mod kept');",
    )
    .unwrap();
}

/// Assert the command succeeded and return its stdout as JSON.
fn json_of(output: &std::process::Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "command failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_json_stdout(output)
}

#[test]
fn governance_config_reports_the_repository_as_json() {
    let (_tmp, data_dir) = temp_data_dir();
    let value = json_of(&run_agora(&data_dir, &["governance", "config", "--json"]));
    let repository = value["repository"].as_str().unwrap_or_default();
    assert!(
        repository.contains('/'),
        "repository should be owner/repo: {value}"
    );
}

#[test]
fn governance_summary_and_events_read_the_compiled_registry() {
    let (_tmp, data_dir) = temp_data_dir();
    seed_governance_registry(&data_dir);

    let summary = json_of(&run_seeded(
        &data_dir,
        &["governance", "summary", "test-mod", "--json"],
    ));
    assert_eq!(summary["raw_upvotes"], 10);
    assert_eq!(summary["vote_issue_number"], 42);

    let events = json_of(&run_seeded(
        &data_dir,
        &["governance", "events", "--item", "test-mod", "--json"],
    ));
    assert_eq!(events.as_array().map(Vec::len), Some(2));

    let limited = json_of(&run_seeded(
        &data_dir,
        &["governance", "events", "--limit", "1", "--json"],
    ));
    assert_eq!(limited.as_array().map(Vec::len), Some(1));
}

#[test]
fn governance_summary_for_an_unknown_item_is_null_not_an_error() {
    let (_tmp, data_dir) = temp_data_dir();
    seed_governance_registry(&data_dir);
    let value = json_of(&run_seeded(
        &data_dir,
        &["governance", "summary", "no-such-mod", "--json"],
    ));
    assert!(value.is_null(), "expected null, got {value}");
}

#[test]
fn governance_events_refuses_an_out_of_range_limit() {
    let (_tmp, data_dir) = temp_data_dir();
    seed_governance_registry(&data_dir);
    // 0 would reach SQLite as LIMIT 0, and a negative value as "no limit".
    for limit in ["0", "-1", "100000"] {
        let output = run_seeded(&data_dir, &["governance", "events", "--limit", limit]);
        assert!(
            !output.status.success(),
            "--limit {limit} should be refused"
        );
    }
}

#[test]
fn governance_diagnostics_lists_its_checks_with_github_turned_off() {
    let (_tmp, data_dir) = temp_data_dir();
    seed_governance_registry(&data_dir);
    // Turn GitHub requests off so the network checks cannot reach the network
    // from a test. They then report their own failure, which is still a check.
    let set = run_seeded(
        &data_dir,
        &["settings", "set", "network_github_oauth_enabled", "false"],
    );
    assert!(set.status.success());

    // Exit status depends on the machine's sign-in and build config, so only
    // the shape of the output is asserted here.
    let output = run_seeded(&data_dir, &["governance", "diagnostics", "--json"]);
    let value = assert_json_stdout(&output);
    let ids: Vec<&str> = value
        .as_array()
        .expect("diagnostics is a JSON array")
        .iter()
        .filter_map(|check| check["id"].as_str())
        .collect();
    for expected in ["governance_repo_resolved", "repository_metadata_readable"] {
        assert!(ids.contains(&expected), "missing {expected}: {ids:?}");
    }
}

#[test]
fn registry_review_queries_read_the_seeded_registry() {
    let (_tmp, data_dir) = temp_data_dir();
    seed_governance_registry(&data_dir);

    let under_review = json_of(&run_seeded(
        &data_dir,
        &["registry", "under-review", "--json"],
    ));
    let ids: Vec<&str> = under_review
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert_eq!(ids, vec!["flagged-mod"]);

    let reviews = json_of(&run_seeded(
        &data_dir,
        &["registry", "reviews", "flagged-mod", "--json"],
    ));
    assert_eq!(reviews[0]["text"], "Crashes on load");
    assert_eq!(reviews[0]["issue_number"], 77);

    let audit = json_of(&run_seeded(&data_dir, &["registry", "audit-log", "--json"]));
    assert_eq!(audit.as_array().map(Vec::len), Some(1));

    let resolutions = json_of(&run_seeded(
        &data_dir,
        &["registry", "resolutions", "--json"],
    ));
    assert_eq!(resolutions[0]["action"], "triage_keep");

    let zero = run_seeded(&data_dir, &["registry", "audit-log", "--limit", "0"]);
    assert!(!zero.status.success(), "--limit 0 should be refused");
}

#[test]
fn registry_review_queries_fail_without_a_registry() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["registry", "under-review"]);
    assert!(
        !output.status.success(),
        "no registry.db should be an error, not an empty list"
    );
}

#[test]
fn technic_search_is_refused_while_technic_consent_is_off() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["technic", "search", "tekkit"]);
    assert!(
        !output.status.success(),
        "technic search must not run with consent off"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panic"), "must not panic: {stderr}");
}

#[test]
fn technic_install_is_refused_while_technic_consent_is_off() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["technic", "install", "tekkit"]);
    assert!(!output.status.success());
    let listed = run_agora(&data_dir, &["list-instances", "--json"]);
    assert_eq!(assert_json_stdout(&listed), serde_json::json!([]));
}

#[test]
fn technic_search_rejects_a_zero_limit() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(&data_dir, &["technic", "search", "x", "--limit", "0"]);
    assert!(!output.status.success());
}

/// A pin that cannot match a SHA-256 digest is refused before any install
/// starts, rather than installing with a hash that looks like protection.
#[test]
fn technic_install_refuses_a_malformed_sha256_pin() {
    let (_tmp, data_dir) = temp_data_dir();
    let non_hex = "g".repeat(64);
    for bad in ["abc", non_hex.as_str(), ""] {
        let output = run_agora(
            &data_dir,
            &["technic", "install", "tekkit", "--sha256", bad],
        );
        assert!(
            !output.status.success(),
            "--sha256 {bad:?} should be refused"
        );
    }
    let listed = run_agora(&data_dir, &["list-instances", "--json"]);
    assert_eq!(assert_json_stdout(&listed), serde_json::json!([]));
}

#[test]
fn pack_install_provider_needs_a_project_and_excludes_a_manifest() {
    let (_tmp, data_dir) = temp_data_dir();
    let no_project = run_agora(&data_dir, &["pack", "install", "--provider", "modrinth"]);
    assert!(!no_project.status.success(), "--provider needs --project");

    let mixed = run_agora(
        &data_dir,
        &[
            "pack",
            "install",
            "--provider",
            "modrinth",
            "--project",
            "abc",
            "some.json",
            "inst",
        ],
    );
    assert!(
        !mixed.status.success(),
        "a manifest and --provider conflict"
    );

    let neither = run_agora(&data_dir, &["pack", "install"]);
    assert!(!neither.status.success(), "pack install needs an input");
}

#[test]
fn pack_install_with_an_unknown_provider_is_refused() {
    let (_tmp, data_dir) = temp_data_dir();
    let output = run_agora(
        &data_dir,
        &[
            "pack",
            "install",
            "--provider",
            "no-such-provider",
            "--project",
            "abc",
        ],
    );
    assert!(!output.status.success());
}

/// Write a development plugin with one boolean setting and no capabilities,
/// so it installs without a consent prompt. Returns its folder.
fn write_settings_plugin(root: &Path) -> PathBuf {
    let dir = root.join("settings-probe");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("agora-plugin.json"),
        r#"{
  "manifest": 1,
  "id": "agora.settings-probe",
  "name": "Settings probe",
  "version": "0.1.0",
  "license": "MIT",
  "apiRange": ">=0.1, <0.2",
  "entrypoint": "main.js",
  "activation": [],
  "capabilities": { "required": [] },
  "contributions": {
    "settings": [{ "key": "notify", "title": "Notify", "type": "boolean", "default": false }]
  }
}"#,
    )
    .unwrap();
    std::fs::write(dir.join("main.js"), "export function noop() {}\n").unwrap();
    dir
}

#[test]
fn plugin_settings_show_set_and_refuse_bad_values() {
    let (_tmp, data_dir) = temp_data_dir();
    let plugin_root = tempdir();
    let plugin_src = write_settings_plugin(plugin_root.path());
    let plugin_src = plugin_src.to_string_lossy().to_string();
    let installed = run_agora(
        &data_dir,
        &[
            "plugin",
            "install",
            "--yes",
            "development",
            plugin_src.as_str(),
        ],
    );
    assert!(
        installed.status.success(),
        "install failed: {}",
        String::from_utf8_lossy(&installed.stderr)
    );

    let before = json_of(&run_agora(
        &data_dir,
        &["plugin", "settings", "agora.settings-probe", "--json"],
    ));
    assert_eq!(before["values"]["notify"], false);
    assert_eq!(before["definitions"].as_array().map(Vec::len), Some(1));

    let set = run_agora(
        &data_dir,
        &["plugin", "set", "agora.settings-probe", "notify", "true"],
    );
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let after = json_of(&run_agora(
        &data_dir,
        &["plugin", "settings", "agora.settings-probe", "--json"],
    ));
    assert_eq!(after["values"]["notify"], true);

    // Wrong type: a quoted string where a boolean is declared.
    let wrong_type = run_agora(
        &data_dir,
        &["plugin", "set", "agora.settings-probe", "notify", "\"yes\""],
    );
    assert!(
        !wrong_type.status.success(),
        "a wrong-typed value must be refused"
    );

    // Undeclared key.
    let unknown_key = run_agora(
        &data_dir,
        &["plugin", "set", "agora.settings-probe", "nope", "true"],
    );
    assert!(
        !unknown_key.status.success(),
        "an undeclared key must be refused"
    );

    // The stored value did not change after the refusals.
    let still = json_of(&run_agora(
        &data_dir,
        &["plugin", "settings", "agora.settings-probe", "--json"],
    ));
    assert_eq!(still["values"]["notify"], true);
}

#[test]
fn plugin_settings_for_a_missing_plugin_fail() {
    let (_tmp, data_dir) = temp_data_dir();
    let read = run_agora(&data_dir, &["plugin", "settings", "agora.nothing-here"]);
    assert!(!read.status.success());
    let write = run_agora(
        &data_dir,
        &["plugin", "set", "agora.nothing-here", "k", "true"],
    );
    assert!(!write.status.success());
}

// ---------------------------------------------------------------------------
// pack export / review / import, backups, launcher import
// ---------------------------------------------------------------------------

#[test]
fn pack_export_mrpack_writes_a_pack_file() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "px-mr", &["sodium.jar"]);
    let out = data_dir.join("px-mr.mrpack");
    let out_arg = out.to_string_lossy().to_string();

    let output = run_agora_json(
        &data_dir,
        &[
            "pack", "export", "px-mr", "--format", "mrpack", "--out", &out_arg,
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed = assert_json_stdout(&output);
    assert_eq!(parsed["status"], "exported");
    assert!(out.is_file(), "the .mrpack should exist");
}

#[test]
fn pack_export_agora_refuses_sourceless_mods_without_yes() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "px-agora", &["sodium.jar"]);
    let out = data_dir.join("px.agora-pack.json");
    let out_arg = out.to_string_lossy().to_string();
    let args = [
        "pack", "export", "px-agora", "--format", "agora", "--out", &out_arg,
    ];

    let refused = run_agora(&data_dir, &args);
    assert!(!refused.status.success(), "must refuse without --yes");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("sodium.jar"), "names the mod: {stderr}");
    assert!(!out.exists(), "nothing is written on refusal");

    let mut with_yes = args.to_vec();
    with_yes.push("--yes");
    let done = run_agora_json(&data_dir, &with_yes);
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    let parsed = assert_json_stdout(&done);
    assert_eq!(parsed["modsWithoutSource"][0], "sodium.jar");
    assert!(out.is_file());
}

#[test]
fn pack_export_lockfile_and_missing_instance() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "px-lock", &["sodium.jar"]);
    let out = data_dir.join("px.lock.json");
    let out_arg = out.to_string_lossy().to_string();
    let ok = run_agora(
        &data_dir,
        &[
            "pack", "export", "px-lock", "--format", "lockfile", "--out", &out_arg,
        ],
    );
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(out.is_file());

    let missing = run_agora(
        &data_dir,
        &[
            "pack", "export", "nope", "--format", "lockfile", "--out", &out_arg,
        ],
    );
    assert!(!missing.status.success());
}

#[test]
fn pack_review_stages_a_copy_and_import_refuses_unknown_or_unfetchable() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "pr-src", &["sodium.jar"]);
    let pack = data_dir.join("pr-src.agora-pack.json");
    let pack_arg = pack.to_string_lossy().to_string();
    let exported = run_agora(
        &data_dir,
        &[
            "pack", "export", "pr-src", "--format", "agora", "--out", &pack_arg, "--yes",
        ],
    );
    assert!(exported.status.success());

    let review = run_agora_json(&data_dir, &["pack", "review", &pack_arg]);
    assert!(
        review.status.success(),
        "{}",
        String::from_utf8_lossy(&review.stderr)
    );
    let parsed = assert_json_stdout(&review);
    let id = parsed["reviewId"].as_str().expect("review id").to_string();
    assert_eq!(id.len(), 64);
    assert!(parsed["entries"].as_array().is_some());

    // A review id nobody reviewed is refused.
    let unknown = run_agora(&data_dir, &["pack", "import", &"0".repeat(64)]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("Review"));

    // The sourceless mod cannot be fetched: the import is refused.
    let refused = run_agora(&data_dir, &["pack", "import", &id]);
    assert!(!refused.status.success());
    assert!(
        !data_dir.join("instances").join("pr-src-2").exists(),
        "a refused import creates nothing"
    );
}

#[test]
fn pack_review_rejects_unknown_file_types() {
    let (_tmp, data_dir) = temp_data_dir();
    let file = data_dir.join("notes.txt");
    std::fs::write(&file, "hi").unwrap();
    let output = run_agora(&data_dir, &["pack", "review", &file.to_string_lossy()]);
    assert!(!output.status.success());
}

#[test]
fn backup_export_import_and_prune_round_trip() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "bk-src", &["sodium.jar"]);
    create_loader_instance(&data_dir, "bk-dst", &["lithium.jar"]);
    let out_dir = data_dir.join("backups");
    let out_arg = out_dir.to_string_lossy().to_string();

    let exported = run_agora_json(
        &data_dir,
        &["backup", "export", "bk-src", "--out", &out_arg],
    );
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let parsed = assert_json_stdout(&exported);
    let artifact = parsed["path"].as_str().expect("artifact path").to_string();
    assert!(Path::new(&artifact).is_file());

    let imported = run_agora_json(
        &data_dir,
        &["backup", "import", &artifact, "--instance", "bk-dst"],
    );
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let snapshots =
        assert_json_stdout(&run_agora_json(&data_dir, &["snapshots", "list", "bk-dst"]));
    assert!(!snapshots.as_array().unwrap().is_empty());

    // Prune keeps the newest and needs --yes without a terminal.
    run_agora(
        &data_dir,
        &["snapshots", "create", "bk-src", "--label", "second"],
    );
    let refused = run_agora(&data_dir, &["backup", "prune", "bk-src", "--keep", "1"]);
    assert!(!refused.status.success(), "prune must not delete unasked");
    let pruned = run_agora_json(
        &data_dir,
        &["backup", "prune", "bk-src", "--keep", "1", "--yes"],
    );
    assert!(
        pruned.status.success(),
        "{}",
        String::from_utf8_lossy(&pruned.stderr)
    );
    assert_eq!(assert_json_stdout(&pruned)["status"], "pruned");
}

#[test]
fn backup_import_rejects_a_file_that_is_not_a_backup() {
    let (_tmp, data_dir) = temp_data_dir();
    create_loader_instance(&data_dir, "bk-bad", &["sodium.jar"]);
    let junk = data_dir.join("junk.zip");
    std::fs::write(&junk, b"not a zip").unwrap();
    let output = run_agora(
        &data_dir,
        &[
            "backup",
            "import",
            &junk.to_string_lossy(),
            "--instance",
            "bk-bad",
        ],
    );
    assert!(!output.status.success());
}

#[test]
fn import_launchers_lists_and_plan_refuses_unknown_instance() {
    let (_tmp, data_dir) = temp_data_dir();
    let empty_root = data_dir.join("no-launcher-here");
    std::fs::create_dir_all(&empty_root).unwrap();
    let root = empty_root.to_string_lossy().to_string();

    let listed = run_agora_json(&data_dir, &["import", "launchers", "--root", &root]);
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    assert!(assert_json_stdout(&listed).get("prism").is_some());

    let plan = run_agora(
        &data_dir,
        &["import", "plan", "zz-no-such-instance", "--root", &root],
    );
    assert!(!plan.status.success());
    assert!(String::from_utf8_lossy(&plan.stderr).contains("No launcher instance"));
}

/// An instance with one bundled mod and one switched-off mod. Importing its
/// pack needs the loader, which Agora downloads, so the tests that import it
/// are `#[ignore]`d (run them with `--ignored` on a machine with network).
fn create_offline_pack_source(data_dir: &Path, id: &str) {
    create_loader_instance(data_dir, id, &["bundled.jar", "off.jar"]);
    let dir = data_dir.join("instances").join(id);
    std::fs::rename(
        dir.join("mods").join("off.jar"),
        dir.join("mods").join("off.jar.disabled"),
    )
    .unwrap();
    let manifest_path = dir.join("instance_manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["minecraft_version"] = "1.20.1".into();
    manifest["loader_version"] = "0.15.0".into();
    for m in manifest["mods"].as_array_mut().unwrap() {
        if m["filename"] == "off.jar" {
            m["enabled"] = false.into();
        }
    }
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

fn assert_imported_pack_keeps_both_mods(data_dir: &Path, new_id: &str) {
    let dir = data_dir.join("instances").join(new_id);
    assert!(dir.is_dir(), "the new instance should exist");
    assert!(dir.join("mods").join("bundled.jar").is_file());
    assert!(
        dir.join("mods").join("off.jar.disabled").is_file(),
        "the disabled mod stays disabled"
    );
    assert!(!dir.join("mods").join("off.jar").exists());
}

fn assert_review_copy_cleaned_up(data_dir: &Path, review_id: &str) {
    let staged = agora_core::app_paths::AppPaths::from_root(data_dir.to_path_buf())
        .staging_root()
        .join("pack-review")
        .join(review_id);
    assert!(!staged.exists(), "the staged copy should be removed");
}

#[test]
#[ignore = "downloads the Fabric loader; run with --ignored"]
fn pack_review_then_import_of_an_mrpack_keeps_both_mods() {
    let (_tmp, data_dir) = temp_data_dir();
    create_offline_pack_source(&data_dir, "mr-src");
    let pack = data_dir.join("mr-src.mrpack");
    let pack_arg = pack.to_string_lossy().to_string();
    let exported = run_agora(
        &data_dir,
        &[
            "pack", "export", "mr-src", "--format", "mrpack", "--out", &pack_arg,
        ],
    );
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );

    let review = run_agora_json(&data_dir, &["pack", "review", &pack_arg]);
    assert!(
        review.status.success(),
        "{}",
        String::from_utf8_lossy(&review.stderr)
    );
    let id = assert_json_stdout(&review)["reviewId"]
        .as_str()
        .unwrap()
        .to_string();

    let imported = run_agora_json(&data_dir, &["pack", "import", &id, "--name", "mr-copy"]);
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let new_id = assert_json_stdout(&imported)["instance_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_imported_pack_keeps_both_mods(&data_dir, &new_id);
    assert_review_copy_cleaned_up(&data_dir, &id);
}

#[test]
#[ignore = "fetches Minecraft metadata; run with --ignored"]
fn import_plan_then_run_copies_a_prism_fixture_instance() {
    let (_tmp, data_dir) = temp_data_dir();
    // Outside the data dir: Agora refuses its own folder as an import source.
    let fixture_tmp = tempdir();
    let prism = fixture_tmp.path().join("PrismLauncher");
    let instances = prism.join("instances");
    let mods = instances.join("FixInst").join("minecraft").join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    std::fs::write(
        prism.join("prismlauncher.cfg"),
        "[General]\nInstanceDir=instances\n",
    )
    .unwrap();
    std::fs::write(
        instances.join("FixInst").join("instance.cfg"),
        "[Minecraft]\nname=Fixture Instance\n",
    )
    .unwrap();
    std::fs::write(
        instances.join("FixInst").join("mmc-pack.json"),
        r#"{"components":[{"uid":"net.minecraft","version":"1.21"}]}"#,
    )
    .unwrap();
    std::fs::write(mods.join("a.jar"), b"mod content").unwrap();
    let root = prism.to_string_lossy().to_string();

    let plan = run_agora_json(
        &data_dir,
        &["import", "plan", "prism:FixInst", "--root", &root],
    );
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let parsed = assert_json_stdout(&plan);
    let fingerprint = parsed["batch_fingerprint"].as_str().unwrap().to_string();

    let stale = run_agora(
        &data_dir,
        &[
            "import",
            "run",
            "prism:FixInst",
            "--root",
            &root,
            "--fingerprint",
            "0000",
        ],
    );
    assert!(!stale.status.success(), "a stale fingerprint is refused");

    let run = run_agora_json(
        &data_dir,
        &[
            "import",
            "run",
            "prism:FixInst",
            "--root",
            &root,
            "--fingerprint",
            &fingerprint,
        ],
    );
    assert!(
        run.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let imported = parsed["items"][0]["destination_id"].as_str().unwrap();
    assert!(data_dir
        .join("instances")
        .join(imported)
        .join("mods")
        .join("a.jar")
        .is_file());
}
