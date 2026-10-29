//! Integration tests for the dependency, template, and instance-maintenance
//! commands: `mod graph|orphans|why|disable-plan`, `mod enable --with-deps`,
//! `instance health|revert|loader-version|java|wrapper`, and `template`.
//!
//! Each test runs the compiled `agora` binary against a temporary `--data-dir`
//! and makes no network calls. Instances are written straight to disk, the same
//! way `cli_integration.rs` writes them.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

const AGORA_BIN: &str = env!("CARGO_BIN_EXE_agora");

struct TempRoot(PathBuf);

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_root() -> TempRoot {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "agora-cli-tools-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).expect("create temp data root");
    TempRoot(path)
}

fn agora(data_dir: &Path, args: &[&str]) -> Output {
    Command::new(AGORA_BIN)
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run agora")
}

fn agora_json(data_dir: &Path, args: &[&str]) -> Output {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    agora(data_dir, &all)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_str(&stdout(output))
        .unwrap_or_else(|e| panic!("stdout must be JSON: {e}\n{}", stdout(output)))
}

/// One mod in the manifest. `disabled` writes the jar as `<file>.disabled`.
struct ModFixture {
    filename: &'static str,
    jar_id: &'static str,
    depends_on: &'static [&'static str],
    as_dependency: bool,
    disabled: bool,
}

fn user_mod(
    filename: &'static str,
    jar_id: &'static str,
    depends_on: &'static [&'static str],
) -> ModFixture {
    ModFixture {
        filename,
        jar_id,
        depends_on,
        as_dependency: false,
        disabled: false,
    }
}

fn dependency_mod(filename: &'static str, jar_id: &'static str) -> ModFixture {
    ModFixture {
        filename,
        jar_id,
        depends_on: &[],
        as_dependency: true,
        disabled: false,
    }
}

/// Write a Fabric instance with the given mods. The jars are plain bytes, not
/// real mod archives, so the manifest's dependency fields are what count.
fn create_instance(data_dir: &Path, id: &str, mods: &[ModFixture]) {
    let db_path = data_dir.join("local_state.db");
    if !db_path.exists() {
        agora_core::db::init_local_state_db(&db_path).unwrap();
    }
    let instance_dir = data_dir.join("instances").join(id);
    std::fs::create_dir_all(instance_dir.join("mods")).unwrap();
    for item in mods {
        let name = if item.disabled {
            format!("{}.disabled", item.filename)
        } else {
            item.filename.to_string()
        };
        std::fs::write(instance_dir.join("mods").join(name), b"not a real jar").unwrap();
    }
    let manifest = serde_json::json!({
        "instance_id": id,
        "name": id,
        "minecraft_version": "1.21",
        "loader": "fabric",
        "loader_version": "0.16.0",
        "is_locked": false,
        "mods": mods.iter().map(|m| serde_json::json!({
            "filename": m.filename,
            "source": "manual",
            "version": null,
            "sha256": "0".repeat(64),
            "installed_at": "2024-01-01T00:00:00Z",
            "java_packages": [],
            "modrinth_id": null,
            "registry_id": null,
            "mod_jar_id": m.jar_id,
            "provided_mod_ids": [],
            "enabled": !m.disabled,
            "content_type": "mod",
            "depends_on": m.depends_on,
            "optional_deps": [],
            "incompatible_deps": [],
            "source_url": null,
            "installed_as_dependency": m.as_dependency,
        })).collect::<Vec<_>>(),
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
            jvm_memory_mode: "auto".into(),
            jvm_gc: "auto".into(),
            jvm_custom_args: String::new(),
            jvm_always_pre_touch: false,
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

fn instance_dir(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join("instances").join(id)
}

/// `a.jar` needs `b.jar` (installed only as a dependency).
fn needs_pair(data_dir: &Path, id: &str) {
    create_instance(
        data_dir,
        id,
        &[
            user_mod("a.jar", "alpha", &["beta"]),
            dependency_mod("b.jar", "beta"),
        ],
    );
}

// ---------------------------------------------------------------------------
// mod graph / orphans / why / disable-plan
// ---------------------------------------------------------------------------

#[test]
fn mod_graph_lists_the_dependency_between_installed_mods() {
    let root = temp_root();
    needs_pair(&root.0, "graph-happy");

    let output = agora_json(&root.0, &["mod", "graph", "graph-happy"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let edges = json(&output);
    let edges = edges.as_array().expect("graph is an array");
    assert_eq!(edges.len(), 1, "one edge expected: {edges:?}");
    assert_eq!(edges[0]["from_filename"], "a.jar");
    assert_eq!(edges[0]["to_filename"], "b.jar");
    assert_eq!(edges[0]["requirement"], "required");
}

#[test]
fn mod_graph_refuses_an_unknown_instance() {
    let root = temp_root();
    let output = agora(&root.0, &["mod", "graph", "no-such-instance"]);
    assert!(
        !output.status.success(),
        "graph of a missing instance must fail"
    );
    assert!(!stderr(&output).is_empty());
}

#[test]
fn mod_orphans_lists_a_dependency_nothing_needs() {
    let root = temp_root();
    create_instance(
        &root.0,
        "orphans-happy",
        &[
            user_mod("a.jar", "alpha", &["beta"]),
            dependency_mod("b.jar", "beta"),
            dependency_mod("c.jar", "gamma"),
        ],
    );

    let output = agora_json(&root.0, &["mod", "orphans", "orphans-happy"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let orphans = json(&output);
    let names: Vec<&str> = orphans
        .as_array()
        .expect("orphans is an array")
        .iter()
        .map(|o| o["filename"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        names,
        vec!["c.jar"],
        "only the unneeded dependency is an orphan"
    );
}

#[test]
fn mod_orphans_refuses_an_unknown_instance() {
    let root = temp_root();
    let output = agora(&root.0, &["mod", "orphans", "no-such-instance"]);
    assert!(!output.status.success());
}

#[test]
fn mod_why_traces_a_dependency_back_to_the_mod_that_needs_it() {
    let root = temp_root();
    needs_pair(&root.0, "why-happy");

    let output = agora(&root.0, &["mod", "why", "why-happy", "b.jar"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Installed to satisfy another mod"), "{text}");
    assert!(
        text.contains("a.jar -> b.jar"),
        "path missing from:\n{text}"
    );
}

#[test]
fn mod_why_refuses_a_file_that_is_not_installed() {
    let root = temp_root();
    needs_pair(&root.0, "why-missing");

    let output = agora(&root.0, &["mod", "why", "why-missing", "ghost.jar"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("No installed file named 'ghost.jar'"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn mod_disable_plan_lists_dependents_and_changes_nothing() {
    let root = temp_root();
    needs_pair(&root.0, "plan-happy");

    let output = agora(&root.0, &["mod", "disable-plan", "plan-happy", "b.jar"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("a.jar"), "dependent missing from:\n{text}");
    assert!(text.contains("Nothing was changed"), "{text}");
    assert!(
        instance_dir(&root.0, "plan-happy")
            .join("mods/b.jar")
            .exists(),
        "a disable plan must not move any file"
    );
}

#[test]
fn mod_disable_plan_refuses_a_file_that_is_not_installed() {
    let root = temp_root();
    needs_pair(&root.0, "plan-missing");

    let output = agora(
        &root.0,
        &["mod", "disable-plan", "plan-missing", "ghost.jar"],
    );
    assert!(!output.status.success());
}

// ---------------------------------------------------------------------------
// mod enable --with-deps
// ---------------------------------------------------------------------------

#[test]
fn mod_enable_with_deps_re_enables_the_disabled_dependency() {
    let root = temp_root();
    create_instance(
        &root.0,
        "enable-deps",
        &[
            ModFixture {
                disabled: true,
                ..user_mod("a.jar", "alpha", &["beta"])
            },
            ModFixture {
                disabled: true,
                ..dependency_mod("b.jar", "beta")
            },
        ],
    );

    let output = agora(
        &root.0,
        &["mod", "enable", "enable-deps", "a.jar", "--with-deps"],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("Also enabled required dependency b.jar"));

    let mods = instance_dir(&root.0, "enable-deps").join("mods");
    assert!(mods.join("a.jar").exists(), "target must be enabled");
    assert!(
        mods.join("b.jar").exists(),
        "required dependency must be enabled"
    );
    assert!(!mods.join("b.jar.disabled").exists());
}

#[test]
fn mod_enable_without_with_deps_leaves_the_dependency_disabled() {
    let root = temp_root();
    create_instance(
        &root.0,
        "enable-alone",
        &[
            ModFixture {
                disabled: true,
                ..user_mod("a.jar", "alpha", &["beta"])
            },
            ModFixture {
                disabled: true,
                ..dependency_mod("b.jar", "beta")
            },
        ],
    );

    let output = agora(&root.0, &["mod", "enable", "enable-alone", "a.jar"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let mods = instance_dir(&root.0, "enable-alone").join("mods");
    assert!(mods.join("a.jar").exists());
    assert!(
        mods.join("b.jar.disabled").exists(),
        "without --with-deps the dependency must stay off"
    );
}

#[test]
fn mod_enable_with_deps_refuses_a_file_the_manifest_does_not_list() {
    let root = temp_root();
    needs_pair(&root.0, "enable-refuse");

    let output = agora(
        &root.0,
        &["mod", "enable", "enable-refuse", "ghost.jar", "--with-deps"],
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("ghost.jar"),
        "stderr: {}",
        stderr(&output)
    );
}

// ---------------------------------------------------------------------------
// instance health
// ---------------------------------------------------------------------------

#[test]
fn instance_health_all_exits_nonzero_when_an_instance_cannot_be_read() {
    let root = temp_root();
    create_instance(&root.0, "health-broken", &[user_mod("a.jar", "alpha", &[])]);
    // A registered instance whose manifest is gone cannot be checked.
    std::fs::remove_file(instance_dir(&root.0, "health-broken").join("instance_manifest.json"))
        .unwrap();

    let output = agora_json(&root.0, &["instance", "health", "--all"]);
    assert!(
        !output.status.success(),
        "an unreadable instance must fail the scan"
    );
    let rows = json(&output);
    let row = rows
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["instance_id"] == "health-broken"))
        .expect("the broken instance is still listed");
    assert!(
        row["error"].is_string(),
        "its error is reported in-band: {row}"
    );
}

#[test]
fn instance_health_all_reports_every_instance() {
    let root = temp_root();
    create_instance(&root.0, "health-one", &[user_mod("a.jar", "alpha", &[])]);
    create_instance(&root.0, "health-two", &[user_mod("a.jar", "alpha", &[])]);

    // The exit code follows the health result; the report is printed either way.
    let output = agora_json(&root.0, &["instance", "health", "--all"]);
    let rows = json(&output);
    let mut ids: Vec<&str> = rows
        .as_array()
        .expect("health --all is an array")
        .iter()
        .map(|r| r["instance_id"].as_str().unwrap_or_default())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["health-one", "health-two"]);
}

#[test]
fn instance_health_without_an_instance_or_all_is_a_usage_error() {
    let root = temp_root();
    let output = agora(&root.0, &["instance", "health"]);
    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
}

#[test]
fn instance_health_for_one_instance_prints_a_score() {
    let root = temp_root();
    create_instance(&root.0, "health-single", &[user_mod("a.jar", "alpha", &[])]);

    let output = agora_json(&root.0, &["instance", "health", "health-single"]);
    let report = json(&output);
    assert!(
        report.get("score").is_some(),
        "report has a score: {report}"
    );
}

#[test]
fn instance_health_for_an_unknown_instance_fails() {
    let root = temp_root();
    let output = agora(&root.0, &["instance", "health", "no-such-instance"]);
    assert!(!output.status.success());
}

// ---------------------------------------------------------------------------
// instance revert
// ---------------------------------------------------------------------------

#[test]
fn instance_revert_restores_the_mods_from_the_lock_snapshot() {
    let root = temp_root();
    create_instance(&root.0, "revert-happy", &[user_mod("a.jar", "alpha", &[])]);

    let lock = agora(&root.0, &["instance", "lock", "revert-happy"]);
    assert!(lock.status.success(), "lock failed: {}", stderr(&lock));
    let unlock = agora(&root.0, &["instance", "unlock", "revert-happy"]);
    assert!(
        unlock.status.success(),
        "unlock failed: {}",
        stderr(&unlock)
    );

    let mods = instance_dir(&root.0, "revert-happy").join("mods");
    std::fs::remove_file(mods.join("a.jar")).unwrap();

    let output = agora(&root.0, &["instance", "revert", "revert-happy"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        mods.join("a.jar").exists(),
        "the lock snapshot's mod must come back"
    );
}

#[test]
fn instance_revert_refuses_an_instance_that_was_never_locked() {
    let root = temp_root();
    create_instance(
        &root.0,
        "revert-never-locked",
        &[user_mod("a.jar", "alpha", &[])],
    );

    let output = agora(&root.0, &["instance", "revert", "revert-never-locked"]);
    assert!(!output.status.success(), "no lock snapshot means no revert");
}

// ---------------------------------------------------------------------------
// instance loader-version
// ---------------------------------------------------------------------------

#[test]
fn instance_loader_version_refuses_a_target_that_is_not_compatible() {
    let root = temp_root();
    create_instance(&root.0, "loader-bogus", &[user_mod("a.jar", "alpha", &[])]);

    let output = agora(
        &root.0,
        &[
            "instance",
            "loader-version",
            "loader-bogus",
            "0.0.1-nope",
            "--yes",
        ],
    );
    assert!(
        !output.status.success(),
        "an unknown loader version must be refused"
    );
    let manifest = std::fs::read_to_string(
        instance_dir(&root.0, "loader-bogus").join("instance_manifest.json"),
    )
    .unwrap();
    assert!(
        !manifest.contains("0.0.1-nope"),
        "a refused switch must not write the version"
    );
}

// ---------------------------------------------------------------------------
// instance java
// ---------------------------------------------------------------------------

#[test]
fn instance_java_sets_memory_and_jvm_args() {
    let root = temp_root();
    create_instance(&root.0, "java-happy", &[]);

    let output = agora_json(
        &root.0,
        &[
            "instance",
            "java",
            "java-happy",
            "--memory",
            "6144",
            "--jvm-args",
            "-Dfoo=bar",
        ],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let row = json(&output);
    assert_eq!(row["memoryMb"], 6144);
    assert_eq!(row["memoryMode"], "manual");
    assert_eq!(row["jvmArgs"], "-Dfoo=bar");
}

#[test]
fn instance_java_accepts_an_existing_executable_as_the_path() {
    let root = temp_root();
    create_instance(&root.0, "java-path", &[]);
    // Any file that exists will do for the path check; this test binary is one.
    let exe = std::env::current_exe().unwrap();

    let output = agora_json(
        &root.0,
        &[
            "instance",
            "java",
            "java-path",
            "--path",
            exe.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(json(&output)["javaPath"], exe.to_str().unwrap());
}

#[test]
fn instance_java_refuses_a_path_that_is_not_a_file() {
    let root = temp_root();
    create_instance(&root.0, "java-bad", &[]);

    let output = agora(
        &root.0,
        &[
            "instance",
            "java",
            "java-bad",
            "--path",
            "Z:/definitely/not/here/java.exe",
        ],
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("No Java executable"),
        "stderr: {}",
        stderr(&output)
    );
}

// ---------------------------------------------------------------------------
// instance wrapper
// ---------------------------------------------------------------------------

#[test]
fn instance_wrapper_sets_shows_and_clears_the_command() {
    let root = temp_root();
    create_instance(&root.0, "wrap-happy", &[]);

    let set = agora(
        &root.0,
        &["instance", "wrapper", "wrap-happy", "mangohud --dlsym"],
    );
    assert!(set.status.success(), "stderr: {}", stderr(&set));

    let shown = agora(&root.0, &["instance", "wrapper", "wrap-happy"]);
    assert!(
        stdout(&shown).contains("Wrapper: mangohud --dlsym"),
        "{}",
        stdout(&shown)
    );

    let clear = agora(&root.0, &["instance", "wrapper", "wrap-happy", "--clear"]);
    assert!(clear.status.success(), "stderr: {}", stderr(&clear));
    let shown = agora(&root.0, &["instance", "wrapper", "wrap-happy"]);
    assert!(
        stdout(&shown).contains("Wrapper: none"),
        "{}",
        stdout(&shown)
    );
}

#[test]
fn instance_wrapper_refuses_an_unmatched_quote() {
    let root = temp_root();
    create_instance(&root.0, "wrap-quote", &[]);

    let output = agora(
        &root.0,
        &["instance", "wrapper", "wrap-quote", "mangohud \"unclosed"],
    );
    assert!(
        !output.status.success(),
        "a malformed wrapper must be rejected up front"
    );
}

// ---------------------------------------------------------------------------
// instance window, server
// ---------------------------------------------------------------------------

#[test]
fn instance_window_sets_shows_and_clears_the_size() {
    let root = temp_root();
    create_instance(&root.0, "win", &[]);

    let set = agora(&root.0, &["instance", "window", "win", "1280x720"]);
    assert!(set.status.success(), "stderr: {}", stderr(&set));
    assert!(
        stdout(&set).contains("Window: 1280x720"),
        "{}",
        stdout(&set)
    );

    let too_small = agora(&root.0, &["instance", "window", "win", "10x10"]);
    assert!(!too_small.status.success());

    let clear = agora(&root.0, &["instance", "window", "win", "--clear"]);
    assert!(
        stdout(&clear).contains("Window: global default"),
        "{}",
        stdout(&clear)
    );
}

#[test]
fn server_add_list_and_remove() {
    let root = temp_root();
    create_instance(&root.0, "srv", &[]);

    let add = agora(
        &root.0,
        &[
            "server",
            "add",
            "srv",
            "play.example.net:25570",
            "--name",
            "Home",
        ],
    );
    assert!(add.status.success(), "stderr: {}", stderr(&add));
    let listed = agora(&root.0, &["server", "list", "srv"]);
    assert!(
        stdout(&listed).contains("play.example.net:25570"),
        "{}",
        stdout(&listed)
    );
    assert!(stdout(&listed).contains("Home"), "{}", stdout(&listed));

    let remove = agora(&root.0, &["server", "remove", "srv", "1"]);
    assert!(remove.status.success(), "stderr: {}", stderr(&remove));
    assert!(stdout(&agora(&root.0, &["server", "list", "srv"])).contains("No servers."));

    let worlds = agora(&root.0, &["world", "list", "srv"]);
    assert!(
        stdout(&worlds).contains("No worlds."),
        "{}",
        stdout(&worlds)
    );
}

// ---------------------------------------------------------------------------
// template
// ---------------------------------------------------------------------------

#[test]
fn template_list_on_a_fresh_root_is_empty() {
    let root = temp_root();
    let output = agora(&root.0, &["template", "list"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("No templates saved."));
}

#[test]
fn template_jvm_profile_round_trips_through_apply_and_delete() {
    let root = temp_root();
    create_instance(&root.0, "tpl-target", &[]);

    let created = agora_json(
        &root.0,
        &[
            "template",
            "create",
            "Big heap",
            "--memory",
            "8192",
            "--gc",
            "low_latency",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let id = json(&created)["id"]
        .as_str()
        .expect("template id")
        .to_string();

    let listed = agora_json(&root.0, &["template", "list"]);
    assert!(stdout(&listed).contains(&id), "list shows the new template");

    let applied = agora(&root.0, &["template", "apply", &id, "tpl-target"]);
    assert!(applied.status.success(), "stderr: {}", stderr(&applied));
    assert!(
        stdout(&applied).contains("JVM settings changed"),
        "{}",
        stdout(&applied)
    );

    let deleted = agora(&root.0, &["template", "delete", &id, "--yes"]);
    assert!(deleted.status.success(), "stderr: {}", stderr(&deleted));
    assert!(stdout(&agora(&root.0, &["template", "list"])).contains("No templates saved."));
}

#[test]
fn template_captures_chosen_files_and_applies_them_to_another_instance() {
    let root = temp_root();
    create_instance(&root.0, "tpl-source", &[]);
    create_instance(&root.0, "tpl-dest", &[]);
    let config = instance_dir(&root.0, "tpl-source").join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("sodium.json"), b"{\"fps\":240}").unwrap();

    let created = agora_json(
        &root.0,
        &[
            "template",
            "create",
            "Sodium",
            "--from",
            "tpl-source",
            "--include",
            "config/sodium.json",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let created = json(&created);
    assert_eq!(created["files"].as_array().map(Vec::len), Some(1));
    let id = created["id"].as_str().expect("template id").to_string();

    let applied = agora(&root.0, &["template", "apply", &id, "tpl-dest"]);
    assert!(applied.status.success(), "stderr: {}", stderr(&applied));
    let copied = instance_dir(&root.0, "tpl-dest").join("config/sodium.json");
    assert_eq!(std::fs::read(copied).unwrap(), b"{\"fps\":240}");
}

#[test]
fn template_create_refuses_nothing_to_capture() {
    let root = temp_root();
    let output = agora(&root.0, &["template", "create", "Empty"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Nothing to capture"),
        "stderr: {}",
        stderr(&output)
    );
}

#[test]
fn template_include_without_from_is_a_usage_error() {
    let root = temp_root();
    let output = agora(
        &root.0,
        &["template", "create", "Loose", "--include", "config/a.json"],
    );
    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
}

#[test]
fn template_delete_without_yes_refuses_when_nobody_can_answer() {
    let root = temp_root();
    let created = agora_json(
        &root.0,
        &["template", "create", "Keep me", "--memory", "4096"],
    );
    let id = json(&created)["id"]
        .as_str()
        .expect("template id")
        .to_string();

    // stdin is closed here, so the confirmation cannot be answered.
    let output = agora(&root.0, &["template", "delete", &id]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("needs confirmation"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(
        stdout(&agora(&root.0, &["template", "list"])).contains(&id),
        "nothing was deleted"
    );
}
