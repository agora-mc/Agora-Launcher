# Context

Agora is an open-source Minecraft mod launcher. Rust workspace: `agora-core` holds all business
logic; three adapters (Tauri desktop app, `agora` CLI, an MCP server) call into it. Architecture
rule is enforced by a script: core must not depend on `tauri`/`clap`/MCP types; platform primitives
are traits in core implemented by adapters. Target: a public launch candidate. Data lives in
`<data_dir>/instances/<instance_id>/` — a Minecraft game directory (`mods/`, `config/`, `saves/`,
`options.txt`, plus a private `.agora_snapshots/` object store and `instance_manifest.json`).

An external review flagged three P1 defects in the snapshot/restore subsystem. I have confirmed
R1 and R3 in the code myself. I want your judgement on the fix design before I write it — including
telling me if my framing of any of these is wrong.

## R1 — restoring a pre-launch snapshot deletes the player's worlds

Snapshots are taken at two different *scopes*. The full scope includes `saves/` (worlds); the
mandatory pre-launch snapshot deliberately excludes `saves/` for performance:

```rust
pub(crate) const TRACKED_ENTRIES: &[&str] = &[
    "mods", "config", "resourcepacks", "shaderpacks", "datapacks",
    "saves", "options.txt", "instance_manifest.json",
];

/// World data (`saves/`) is deliberately excluded: it changes on every game
/// session, so including it forced the pre-launch path to re-walk and re-hash
/// potentially gigabytes of world files each launch.
const PRELAUNCH_TRACKED_ENTRIES: &[&str] = &[
    "mods", "config", "resourcepacks", "shaderpacks", "datapacks",
    "options.txt", "instance_manifest.json",
];

/// Stable identity for the exact roots covered by a snapshot receipt.
fn snapshot_scope_id(entries: &[&str]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"agora-snapshot-scope-v1");
    for entry in entries {
        hasher.update((entry.len() as u64).to_le_bytes());
        hasher.update(entry.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}
```

`snapshot_scope_id` today is used ONLY to key the pre-launch *reuse receipt* (a metadata
fingerprint stored in a sidecar file, so an unchanged instance can reuse the previous pre-launch
snapshot instead of re-hashing). It is NOT stored in the snapshot manifest.

The manifest, which is the only thing restore reads:

```rust
const SNAPSHOT_SCHEMA_VERSION: u32 = 3;

#[derive(Serialize, Deserialize)]
struct SnapshotManifest {
    #[serde(default = "legacy_snapshot_schema_version")]
    schema_version: u32,
    snapshot: Snapshot,            // id, label, created_at, file_count, size_estimate
    files: Vec<SnapshotFileEntry>, // relative_path, size, sha256, blob_sha256
}
```

Contents are stored as SHA-256-named blobs in a shared per-instance object store; the manifest is
just a file list. Legacy ZIP snapshots (older format) are still restorable.

Creation is scope-parameterised:

```rust
pub fn create_snapshot(instance_dir: &Path, label: Option<&str>) -> Result<Snapshot, String> {
    create_snapshot_scoped(instance_dir, label, TRACKED_ENTRIES)
}

pub fn create_snapshot_scoped(
    instance_dir: &Path, label: Option<&str>, entries: &[&str],
) -> Result<Snapshot, String> {
    // ... walks `entries`, stores/reuses blobs, writes manifest (schema_version: 3),
    //     writes a live-file-index cache and a scoped metadata fingerprint receipt.
}
```

Restore is NOT scope-parameterised. It iterates the *full* `TRACKED_ENTRIES` when moving live
state aside, but only promotes roots that appear in the manifest's file list:

```rust
fn snapshot_roots(manifest: &SnapshotManifest) -> HashSet<&str> {
    manifest.files.iter().filter_map(|e| e.relative_path.split('/').next()).collect()
}

fn restore_snapshot_impl(
    instance_dir: &Path, snapshot_id: &str, fail_after_promotions: Option<usize>,
) -> Result<(), String> {
    validate_snapshot_id(snapshot_id)?;
    recover_interrupted_restore(instance_dir)?;
    // ... locate manifest or legacy zip, else Err("snapshot not found")

    let restore_id = uuid::Uuid::new_v4().to_string();
    let extract_dir = instance_dir.join(format!(".agora_restore_extract_{restore_id}"));
    fs::create_dir_all(&extract_dir)?;
    // extract_and_verify: rebuild every manifest file into extract_dir from blobs,
    // verifying SHA-256, BEFORE any live mutation. Rejects duplicate/traversal paths.
    let manifest = match extract_and_verify(instance_dir, snapshot_id, &extract_dir) { ... };

    let pre_dir = pre_restore_dir(instance_dir);   // .agora_pre_restore
    if pre_dir.exists() { fs::remove_dir_all(&pre_dir)?; }
    fs::create_dir_all(&pre_dir)?;

    let marker_path = instance_dir.join(RESTORE_MARKER); // .agora_restore_in_progress
    fs::write(&marker_path, b"restore in progress")?;

    // (A) move ALL live tracked roots aside — including saves/
    let mut moved_current = Vec::new();
    for entry_name in TRACKED_ENTRIES {
        let src = instance_dir.join(entry_name);
        if src.exists() {
            let dst = pre_dir.join(entry_name);
            fs::create_dir_all(dst.parent().unwrap())?;
            if let Err(error) = fs::rename(&src, &dst) {
                let rollback = rollback_restore(instance_dir, &pre_dir, &[], &moved_current, &restore_id);
                let _ = fs::remove_dir_all(&extract_dir);
                return Err(combine_restore_error(format!("failed to move current {entry_name} into backup: {error}"), rollback));
            }
            moved_current.push((*entry_name).to_string());
        }
    }

    // (B) promote only roots present in the snapshot — a pre-launch snapshot has no saves/
    let staged_roots = snapshot_roots(&manifest);
    let mut promoted = Vec::new();
    for entry_name in TRACKED_ENTRIES {
        if !staged_roots.contains(*entry_name) { continue; }
        let src = extract_dir.join(entry_name);
        let dst = instance_dir.join(entry_name);
        let promote_result = if fail_after_promotions == Some(promoted.len()) {
            Err("injected restore promotion failure".to_string())
        } else { fs::rename(&src, &dst).map_err(|e| e.to_string()) };
        if let Err(error) = promote_result {
            let rollback = rollback_restore(instance_dir, &pre_dir, &promoted, &moved_current, &restore_id);
            let _ = fs::remove_dir_all(&extract_dir);
            return Err(combine_restore_error(format!("failed to promote restored {entry_name}: {error}"), rollback));
        }
        promoted.push((*entry_name).to_string());
    }

    if marker_path.exists() { fs::remove_file(&marker_path)?; }
    // (C) the backup — now the ONLY copy of saves/ — is deleted
    if pre_dir.exists() { fs::remove_dir_all(&pre_dir)
        .map_err(|e| format!("restore succeeded but backup cleanup failed: {e}"))?; }
    let _ = fs::remove_dir_all(&extract_dir);
    let _ = mark_instance_mutated(instance_dir);
    Ok(())
}
```

So: restoring a pre-launch snapshot moves `saves/` into `.agora_pre_restore`, never promotes it
back, then `remove_dir_all`s the backup. Every world is gone. Interrupted-restore recovery has the
same TRACKED_ENTRIES assumption:

```rust
fn recover_interrupted_restore(instance_dir: &Path) -> Result<(), String> {
    let marker = instance_dir.join(RESTORE_MARKER);
    let pre_dir = pre_restore_dir(instance_dir);
    if !marker.exists() {
        if pre_dir.exists() { fs::remove_dir_all(&pre_dir)?; }   // <-- also deletes an orphaned backup
        return Ok(());
    }
    if !pre_dir.is_dir() { return Err("Previous restore was interrupted without a recovery backup; live state was left untouched.".into()); }
    let backed_up = TRACKED_ENTRIES.iter().filter(|e| pre_dir.join(e).exists()).map(|e| (*e).to_string()).collect::<Vec<_>>();
    if backed_up.is_empty() { fs::remove_file(&marker)?; fs::remove_dir_all(&pre_dir)?; return Ok(()); }
    rollback_restore(instance_dir, &pre_dir, &backed_up, &backed_up, &format!("interrupted-{}", uuid::Uuid::new_v4()))?;
    if pre_dir.exists() { fs::remove_dir_all(&pre_dir)?; }
    Ok(())
}
```

Rollback, for reference (used on failure paths above):

```rust
fn rollback_restore(instance_dir: &Path, pre_dir: &Path, promoted: &[String],
                    moved_current: &[String], restore_id: &str) -> Result<(), String> {
    let failed_dir = instance_dir.join(format!(".agora_failed_restore_{restore_id}"));
    fs::create_dir_all(&failed_dir)?;
    let mut errors = Vec::new();
    for entry_name in promoted.iter().rev() {          // displace partial promotions
        let live = instance_dir.join(entry_name);
        if live.exists() { /* rename live -> failed_dir/entry_name, collecting errors */ }
    }
    for entry_name in moved_current.iter().rev() {     // move backups back
        let backup = pre_dir.join(entry_name);
        let live = instance_dir.join(entry_name);
        if !backup.exists() { errors.push(...); continue; }
        if live.exists() { errors.push("rollback destination still exists"); continue; }
        if let Err(e) = fs::rename(&backup, &live) { errors.push(...); }
    }
    if errors.is_empty() { let _ = fs::remove_dir_all(&failed_dir);
        let _ = fs::remove_file(instance_dir.join(RESTORE_MARKER)); Ok(()) }
    else { Err(format!("rollback incomplete; original data remains in {} and partial data in {}: {}",
        pre_dir.display(), failed_dir.display(), errors.join("; "))) }
}
```

The desktop UI takes an undo snapshot before restoring; the CLI does not (code below).

## R2 — rollback omits paths that operations actually write

Neither `TRACKED_ENTRIES` nor the pre-launch scope covers `defaultconfigs/` or `kubejs/`, and
modpack updates / instance templates can write additional root-level config files. So an install
or pack-update that fails mid-way rolls back to a snapshot that never captured those paths: the
partial write survives the "rollback".

## R3 — restore is not serialized against other instance mutations

Core has a cross-process file lock manager already, used by install/loader/launch/import/etc:

```rust
pub enum LockResource {
    RegistryUpdate,
    LoaderInstall,
    JavaMajor(u32),
    LaunchMaterialize,
    Instance(String),   // per-instance lock for install/remove/update operations
}
impl LockManager {
    pub fn acquire(&self, resource: LockResource, operation: &str) -> LauncherResult<LockGuard>;
    pub fn acquire_with_timeout(&self, /* ..., timeout, cancellation token */) -> LauncherResult<LockGuard>;
}
// LockGuard holds an ownership nonce; Drop only unlinks the file if the nonce still matches.
// Default timeout 30s; distinguishes Contested (live owner) / Stale (dead owner) / Corrupt.
```

`snapshot.rs` itself takes no locks — it is a free-function module over an `instance_dir: &Path`
with no access to `LauncherContext` (which owns `paths` and the `LockManager`).

The Tauri restore command checks only in-process launch state:

```rust
#[tauri::command]
pub async fn restore_snapshot(
    app: tauri::AppHandle, state: tauri::State<'_, LauncherState>,
    instance_id: String, snapshot_id: String,
) -> LauncherResult<()> {
    let sanitized = paths::sanitize_id(&instance_id);
    let instance_dir = paths::instance_dir(&app, &sanitized)?;
    {
        let shared = state.lock().await;
        let direct_active = shared.running_processes.values().any(|p| p.instance_id == sanitized);
        let launch_active = shared.launch_reservations.contains(&sanitized);
        if direct_active || launch_active {
            return Err(LauncherError::Generic { code: "ERR_INSTANCE_RUNNING".into(),
                message: "Stop the running game before restoring this instance.".into() });
        }
    }
    tokio::task::spawn_blocking(move || {
        let pre_label = format!("pre-restore-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
        agora_core::snapshot::create_snapshot(&instance_dir, Some(&pre_label))
            .map_err(|e| format!("Could not create undo snapshot: {e}"))?;
        agora_core::snapshot::restore_snapshot(&instance_dir, &snapshot_id)?;
        agora_core::lkg::run_retention(&instance_dir)?;
        Ok::<(), String>(())
    }).await??;
    Ok(())
}
```

The check is TOCTOU (released before the blocking work starts), it is in-process only (a
concurrent CLI `agora snapshots restore` sees none of it), and no `LockResource::Instance` guard is
held. The CLI path has no checks at all:

```rust
SnapshotsCmd::Restore { instance, snapshot_id } => {
    let instance_dir = agora_core::paths::instance_dir(data_dir, &instance)?;
    if !instance_dir.exists() { anyhow::bail!("Instance '{}' not found", instance); }
    agora_core::snapshot::restore_snapshot(&instance_dir, &snapshot_id)?;
    println!("Restored instance '{}' from snapshot {}", instance, snapshot_id);
}
```

Also note `launch_reservations` is desktop-process in-memory state, so "an instance is launching"
is not visible cross-process; a launch takes `LockResource::Instance` only briefly during
materialization, not for the game's lifetime.

# What I want from you

Design judgement, not code. Be concrete about mechanism and about what breaks.

1. **R1 scope persistence.** My instinct is to bump the manifest to schema v4 with an explicit
   `scope: Vec<String>` (the roots the snapshot covers), and make restore iterate *that* set for
   both the move-aside and the promote passes, so an out-of-scope root like `saves/` is never
   touched. Two things I'm unsure about:
   (a) Back-compat. Existing v1–v3 manifests and legacy ZIPs have no scope field. Inferring scope
   from `snapshot_roots(manifest)` is wrong in the empty-directory case: a full snapshot of an
   instance whose `saves/` happened to be empty yields no `saves/` files, so it would be inferred
   as pre-launch scope and `saves/` would be left alone — which is *safe* but silently changes
   restore semantics for old full snapshots. Is "infer from roots, and when in doubt leave the
   root untouched rather than delete it" the right rule, or should old manifests be handled some
   other way? Is there a case where leaving a root untouched is worse than replacing it?
   (b) Within a scope, restore currently replaces whole roots by rename. A file that exists live
   but is absent from the snapshot inside an in-scope root is correctly removed by that. Is
   whole-root replacement still right once scope is explicit, or does the empty-directory problem
   (a scope root with zero files in the manifest — indistinguishable from "not in scope" without
   the new field) argue for recording the scope roots explicitly *and* allowing them to be empty?
2. **Should the pre-launch scope keep excluding `saves/` at all**, given the cost that exclusion
   has now imposed? The stated reason is re-hashing gigabytes per launch. Is there a cheaper
   correct option (e.g. capturing `saves/` by metadata-only reference, or snapshotting worlds
   copy-on-write), or is scoped-restore genuinely the right fix and the exclusion fine?
3. **R2 coverage.** How would you make "recovery covers everything an operation writes" a property
   the code enforces rather than a list someone must remember to update? Options I see: widen
   TRACKED_ENTRIES to a denylist (capture the whole instance dir except known-huge/derived paths);
   have each operation declare the paths it will touch and snapshot exactly that union; or keep the
   allowlist but add a test that diffs the instance tree before/after each operation and fails when
   a mutated path is outside the snapshot scope. Which of these actually holds up, and what's the
   failure mode of each?
4. **R3 locking.** Where should the lock live? `snapshot.rs` has no context access, so options are
   (i) push restore into a context-aware service (`SnapshotService`) in core that acquires
   `LockResource::Instance` and is called by both adapters, or (ii) have each adapter acquire the
   lock. And separately: how do I make "the game is not running" a cross-process fact? Is a
   filesystem lock held for the game's whole lifetime the right mechanism, or does that create
   worse problems (crashed launcher leaves a stale lock, though the manager does detect dead
   owners)? Note the desktop undo snapshot + restore must be inside one critical section.
5. **Ordering and risk.** I plan R1+R3 together, then R2. Is the scope change safe to ship without
   a migration step for existing snapshots on users' disks? What would you test to be convinced —
   specifically, what tests would catch a *regression* here rather than just the reported bug?

Tell me plainly if any of my proposed directions is wrong or if there's a fifth thing in this code
that is worse than what the review found.
