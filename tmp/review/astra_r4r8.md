# Context

Agora is an open-source Minecraft mod launcher heading for a public launch candidate. Rust
workspace: `agora-core` holds all business logic; three adapters call into it — a Tauri desktop app
(`desktop/src-tauri`), an `agora` CLI, and an MCP server. An enforcement script forbids core from
depending on `tauri`/`clap`/MCP types. Instances live in `<data_dir>/instances/<id>/` and are
Minecraft game directories.

An external review flagged five P1 defects and one P2. I have confirmed all six in the code and
pasted the relevant parts below. I want your judgement on the *fix* for each — mechanism, blast
radius, and what to test — plus a ranking if you think my ordering is wrong. I do not need code.

---

## R5 (P1) — three HTTP categories silently inherit a 30-second timeout

Core has a category-keyed set of pre-built `reqwest::Client`s. Each category declares a timeout and
a response size cap:

```rust
fn timeout(&self) -> Duration {
    match self {
        ClientCategory::MojangMetadata   => Duration::from_secs(30),
        ClientCategory::MojangContent    => Duration::from_secs(120),
        ClientCategory::Loader           => Duration::from_secs(60),
        ClientCategory::Modrinth         => Duration::from_secs(30),
        ClientCategory::Modpack          => Duration::from_secs(5 * 60),
        ClientCategory::GitHub           => Duration::from_secs(30),
        ClientCategory::Microsoft        => Duration::from_secs(30),
        ClientCategory::Registry         => Duration::from_secs(60),
        ClientCategory::AiAssistant      => Duration::from_secs(60),
        ClientCategory::JavaRuntime      => Duration::from_secs(120),
        ClientCategory::PinnedArtifact   => Duration::from_secs(120),
        ClientCategory::ConsentedContent => Duration::from_secs(5 * 60),
    }
}

fn max_response_bytes(&self) -> Option<u64> {
    match self {
        ClientCategory::MojangContent    => Some(200 * 1024 * 1024),
        ClientCategory::Modrinth         => Some(200 * 1024 * 1024),
        ClientCategory::Modpack          => Some(500 * 1024 * 1024),
        ClientCategory::Loader           => Some(100 * 1024 * 1024),
        ClientCategory::Registry         => Some(100 * 1024 * 1024),
        ClientCategory::JavaRuntime      => Some(512 * 1024 * 1024),
        ClientCategory::PinnedArtifact   => Some(100 * 1024 * 1024),
        ClientCategory::ConsentedContent => Some(500 * 1024 * 1024),
        _ => Some(10 * 1024 * 1024),
    }
}
```

But the struct has only **nine** client fields for **twelve** categories, and three categories map
onto the Modrinth client — which was built with the Modrinth 30s timeout:

```rust
pub struct HttpClients {
    mojang_metadata: reqwest::Client, mojang_content: reqwest::Client, loader: reqwest::Client,
    modrinth: reqwest::Client, github: reqwest::Client, microsoft: reqwest::Client,
    registry: reqwest::Client, ai_assistant: reqwest::Client, java_runtime: reqwest::Client,
}

impl HttpClients {
    pub fn new() -> LauncherResult<Self> {
        Ok(Self {
            mojang_metadata: Self::build_client(ClientCategory::MojangMetadata)?,
            // ... one build_client per field; none for Modpack/PinnedArtifact/ConsentedContent
            java_runtime: Self::build_client(ClientCategory::JavaRuntime)?,
        })
    }

    pub fn get(&self, category: ClientCategory) -> &reqwest::Client {
        match category {
            ClientCategory::Modrinth         => &self.modrinth,
            ClientCategory::Modpack          => &self.modrinth,  // declares 5 min, gets 30s
            ClientCategory::PinnedArtifact   => &self.modrinth,  // declares 2 min, gets 30s
            ClientCategory::ConsentedContent => &self.modrinth,  // declares 5 min, gets 30s
            // ... others map 1:1
        }
    }

    fn build_client(category: ClientCategory) -> LauncherResult<reqwest::Client> {
        reqwest::Client::builder()
            .timeout(category.timeout())          // whole-request timeout, not per-read
            .user_agent(category.user_agent())
            .redirect(reqwest::redirect::Policy::none())  // manual per-hop revalidation instead
            .pool_max_idle_per_host(4)
            .build()
    }
}
```

`reqwest`'s `Client::timeout` is a *total request* deadline including the body stream, so a 400 MB
modpack on a slow connection dies at 30s regardless of progress. Note also that
`.timeout(...)` can be overridden per request with `RequestBuilder::timeout`, and that
`Client::builder()` separately offers `connect_timeout` and `read_timeout`.

I see three ways to fix it: (a) add the three missing client fields; (b) collapse the struct into a
`HashMap<ClientCategory, Client>` or an array indexed by category so a new category cannot silently
alias an existing one; (c) keep one client per *host profile* and set `.timeout()` per request from
`category.timeout()`. **Which is right, and is a total-request deadline even the correct shape for
large downloads — or should these categories move to `connect_timeout` + `read_timeout` (stall
detection) instead, so a slow-but-progressing 500 MB download is never killed?** There is a
progress-callback download path (`checked_get_bytes_with_progress`) that streams chunks, so
per-chunk stall detection is implementable.

## R7 (P1) — Lockdown Mode does not cover all outbound requests

There is a user setting, Lockdown Mode, documented as a global kill switch over every per-endpoint
network toggle:

```rust
/// Lockdown Mode (`network_lockdown_enabled`) is a global override that
/// disables every endpoint.
pub fn is_network_enabled(conn: &Connection, key: &str) -> bool {
    if is_lockdown_enabled(conn) { return false; }
    get_setting(conn, key).ok().flatten().map(|v| is_value_enabled(&v)).unwrap_or(true)
}

pub fn is_lockdown_enabled(conn: &Connection) -> bool {
    get_setting(conn, "network_lockdown_enabled")
        .ok().flatten().map(|v| is_value_enabled(&v)).unwrap_or(false)
}

/// Returns `true` for unknown or unexpected shapes (aligned with the fail-open default).
fn is_value_enabled(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::String(s) => s == "true",
        _ => true,
    }
}
```

Enforcement is **not** in the HTTP layer. `http_client.rs` contains no reference to lockdown or to
the settings DB at all. Instead every feature module is expected to call `is_network_enabled`
itself at its own entry point. Modules that do: `modrinth.rs`, `msa.rs`, `registry_sync.rs`,
`ai_assistant.rs`, `migration_report.rs`, `update_cache.rs`, `technic.rs`, and the launch planner
(which has its own separate five-bit `NetworkPolicy` bitmask passed down through the plan).

Modules that do **not**:

1. **GitHub OAuth device-flow login** (`crates/agora-core/src/auth.rs`) — goes through the
   policy-enforcing helpers (`checked_post_form`, `checked_request_with_headers`,
   `ClientCategory::GitHub`) so URL/scheme/host/SSRF checks apply, but nothing consults lockdown.
   So with Lockdown on, "Sign in with GitHub" still reaches `github.com/login/device/code` and
   `api.github.com/user`.
2. **Governance/voting** (`desktop/src-tauri/src/governance.rs`) — builds a raw `reqwest::Client`
   and POSTs directly to `https://api.github.com/graphql` and GETs `api.github.com/repos/...`.
   This bypasses lockdown *and* the entire `check_request_url` policy layer (scheme/port/userinfo/
   IP-literal/allowlist/private-IP checks, per-hop redirect revalidation, size caps).
3. **Install-pipeline artifact staging** (`install_pipeline.rs` → `download.rs`) — e.g.
   `download_pinned_bytes_standalone(url, pinned_host)` and `download_consented_bytes_standalone`.
   These are "standalone" helpers that call `HttpClients::new()` internally to build a fresh client
   set, then use the checked helpers. Policy applies; lockdown does not. They also mean the
   process holds several independent connection pools.

The review that found this initially proposed gating inside `checked_get_bytes`. I think that is
insufficient (it would miss POST and the direct-client governance calls) but also *wrong-layered*.

**Where should this actually be enforced?** Options I see:
(i) a chokepoint in `http_client::checked_send` (the single function every checked helper funnels
through) that takes a policy handle — but core's HTTP layer currently has no DB access and I'd have
to thread a `&Connection` or a policy snapshot into every call site;
(ii) make `HttpClients` itself carry an `Arc<dyn NetworkGate>` set at construction, so *possessing a
client* implies the gate — which also kills the `HttpClients::new()`-inside-a-standalone-helper
pattern, since a gate-less client would no longer be constructible outside tests;
(iii) keep per-module checks and add an enforcement test/lint that fails when a module performs a
request without a lockdown check.

Which of these survives contact with reality? Specifically: is (ii) worth the churn of threading a
gate through every `HttpClients` construction site, and how would you handle the desktop
`governance.rs` raw client — move governance into core behind the gate, or give the adapter a
gated client type it cannot bypass? And is a lockdown that fails *open* on a malformed setting
value (`is_value_enabled` returning `true` for unexpected JSON shapes) acceptable for a
security-facing toggle, or should the lockdown read specifically fail closed?

## R4 (P1) — pack updates ignore Minecraft/loader requirements

Modpack updates apply a new `.mrpack` over an existing instance. The index parser omits the
`dependencies` object entirely:

```rust
#[derive(Deserialize)]
struct MrpackIndex {
    #[serde(default)] name: String,
    #[serde(default, alias = "versionId")] version_id: Option<String>,
    #[serde(default)] files: Vec<MrpackFile>,
    #[serde(default)] overrides: String,
    // no `dependencies` — the mrpack format carries
    // { "minecraft": "1.20.1", "fabric-loader": "0.15.7" } (or forge/quilt/neoforge)
}
```

The update flow never reads or writes the instance's runtime settings:

```rust
pub fn update_pack<F: PackFileFetcher>(
    conn: &rusqlite::Connection, instance_id: &str, instance_dir: &Path,
    mrpack_path: &Path, staged_dir: &Path,
    resolutions: &BTreeMap<String, ConflictResolution>, fetcher: &F, skip_health_scan: bool,
) -> PackUpdateOutcome {
    let preview = preview_pack_update(conn, instance_id, instance_dir, mrpack_path)?; // three-way merge plan
    // every conflict must have a resolution, else Failed("conflicts")
    stage_pack(mrpack_path, staged_dir, fetcher)?;              // download + hash-verify into staging
    let theirs = theirs_from_mrpack(mrpack_path)?;
    let old_manifest = read_manifest(&instance_dir.join("instance_manifest.json"))?;
    let reconciled = reconcile_manifest(&old_manifest, &preview.plan, resolutions, &theirs, staged_dir)?;
    let outcome = apply_merge(conn, instance_id, instance_dir, staged_dir,
                              &preview.plan, resolutions, &reconciled)?;  // snapshots, then commits
    // ... then a read-only health scan; blockers => PackUpdateOutcome::HealthBlocked
}
```

So an instance pinned to Minecraft 1.20.1 + Fabric 0.15.7 can be updated to a pack built for
1.21.1 + NeoForge: every mod file is replaced, but `minecraft_version`, `loader`, and
`loader_version` on the instance row and in the manifest keep their old values. The next launch
runs 1.21 mods on a 1.20.1 client with the wrong loader. The post-commit health scan may or may not
flag it (it checks manifest drift, not runtime compatibility).

The required outcome from the review is: "validate requirements before mutation; perform a
supported migration or reject incompatible updates clearly."

**My question is what "supported migration" should mean here.** The repo already has a separate
`version_migration.rs` and a read-only `migration_report.rs` ("can this instance move to the next
Minecraft version, and what breaks?"), and loader installation is its own locked operation
(`LockResource::LoaderInstall`, pinned loader manifests with SHA-256). Is the right move to:
(a) parse `dependencies`, and *reject* with a clear error whenever they differ from the instance,
telling the user to use the migration path — simplest, but a modpack's whole point is that its
updates carry MC/loader bumps, so this rejects the common case;
(b) parse `dependencies` and, when they differ, run the loader install + version change as part of
the same transaction, rolled back with the rest on failure — correct but couples pack update to
loader installation and network access mid-transaction;
(c) parse them, show the required change in the *preview* (which already exists and already
enumerates conflicts requiring explicit resolution), and require the user to consent to the runtime
change as one more resolution before applying.
Which of these is right for a launcher at this maturity, and what's the failure mode I'm not
seeing? Also: is silently keeping old runtime settings ever safer than failing loudly?

## R6 (P1) — default instance cloning follows source symlinks

```rust
pub struct ClonePrefs {
    pub copy_saves: bool, pub copy_mods: bool, pub copy_resource_packs: bool,
    pub copy_shader_packs: bool, pub copy_screenshots: bool, pub copy_config: bool,
    pub copy_servers: bool, pub copy_options: bool,
    pub use_hard_links: bool,   // default false
    pub use_sym_links: bool,    // default false
}
// Default: every copy_* true, both link flags false.

pub fn clone_instance(src_dir: &Path, dest_dir: &Path, prefs: &ClonePrefs) -> Result<String, String> {
    if !src_dir.is_dir() { return Err(...); }
    let instance_id = paths::sanitize_id(&src_dir.file_name()...);
    if dest_dir.exists() { fs::remove_dir_all(dest_dir)?; }   // note: unconditional
    fs::create_dir_all(dest_dir)?;
    // copies manifest.json and instance_manifest.json if present
    for mapping in DIR_MAPPINGS {            // saves, mods, resourcepacks, shaderpacks,
        if !(mapping.pref)(prefs) { continue; }   // screenshots, config, servers, options
        let src_child = src_dir.join(mapping.dir);
        if !src_child.exists() { continue; }      // `exists()` follows links
        copy_entry(&src_child, &dest_dir.join(mapping.dir), prefs)?;
    }
    Ok(instance_id)
}

fn copy_entry(src: &Path, dst: &Path, prefs: &ClonePrefs) -> Result<(), String> {
    if prefs.use_sym_links && symlink_entry(src, dst) { return Ok(()); }
    if prefs.use_hard_links && src.is_file() && hardlink_entry(src, dst) { return Ok(()); }
    if src.is_dir() {                              // <-- follows symlinks / Windows junctions
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_entry(&entry.path(), &dst.join(entry.file_name()), prefs)?;   // unbounded recursion
        }
    } else if src.is_file() {                      // <-- also follows links
        fs::copy(src, dst)?;
    }
    Ok(())
}
```

`Path::is_dir`/`is_file`/`exists` all follow links, and `fs::read_dir` follows a link to a
directory. So with the *default* (non-link) prefs: a symlink or Windows directory junction inside
`mods/` pointing at `C:\Users\me\Documents` gets deep-copied into the clone; a link pointing at an
ancestor of the instance produces unbounded recursion (stack overflow via the recursive
`copy_entry`, or a full disk). A link to a file outside the instance is silently materialised as a
real copy. Windows reparse points include junctions, which many users have (and OneDrive/Dropbox
placeholder files are also reparse points).

The review says: "detect links/reparse points before recursion; define explicit handling for shared
folders and test escapes and cycles."

**What should the *policy* be, not just the mechanism?** `symlink_metadata()` gives me
`file_type().is_symlink()` cross-platform, and on Windows I can additionally check
`FILE_ATTRIBUTE_REPARSE_POINT` via `MetadataExt::file_attributes()`. But once detected, what do I
do — skip the link with a warning, recreate it as a link in the clone (leaking a shared mutable
directory into a "copy" the user thinks is independent), or refuse the whole clone? Users
*deliberately* symlink shared `resourcepacks/`. Note the `use_sym_links` pref means the user can
already ask for a link-based clone. Also: is a depth/visited-inode cap worth adding as defence in
depth even after link detection, and does the unconditional `fs::remove_dir_all(dest_dir)` at the
top deserve its own finding?

## R8 (P2) — explicit GC selections are silently saved as Auto

The UI offers four GC choices and sends these strings: `auto`, `manual`, `low_latency`,
`high_efficiency` (`desktop/src/lib/tauri.ts`: `type GcProfile = 'low_latency' | 'high_efficiency'
| 'manual'`, plus `'auto'`). Persistence uses a *different* vocabulary:

```rust
// crates/agora-core/src/instance_service.rs — the only write path
pub fn update_jvm(&self, instance_id: &str, memory_mb: i64, gc: &str,
                  always_pre_touch: bool, custom_args: &str, memory_mode: &str) -> LauncherResult<()> {
    let gc = match gc.trim().to_ascii_lowercase().as_str() {
        "auto" | "g1gc" | "zgc" | "shenandoah" | "manual" => gc.trim().to_ascii_lowercase(),
        _ => "auto".to_string(),            // low_latency / high_efficiency land here
    };
    // ... update_instance_jvm(&conn, &instance_id, memory_mb, &gc, ...)
}
```

Both *read* paths use the UI vocabulary and have no idea `shenandoah` exists:

```rust
// launch_service.rs — builds the actual launch arguments
let jvm_gc_profile = match row.jvm_gc.to_ascii_lowercase().as_str() {
    "zgc" | "low_latency"  => Some(GcProfile::LowLatency),
    "high_efficiency"      => Some(GcProfile::HighEfficiency),
    "manual"               => Some(GcProfile::Manual),
    _ => None,                              // None == Auto
};

// models.rs — the preview shown in the editor
let profile = match gc.as_str() {
    "auto" | "" | "g1gc"   => None,
    "low_latency" | "zgc"  => Some(GcProfile::LowLatency),
    "high_efficiency"      => Some(GcProfile::HighEfficiency),
    "manual"               => Some(GcProfile::Manual),
    _ => None,
};
// GcProfile has exactly three variants: LowLatency (Generational ZGC),
// HighEfficiency (Aikar's G1GC), Manual (raw user flags).
```

So picking "ZGC · low latency" saves `auto`; picking "G1GC · high efficiency" saves `auto`. There
are three vocabularies (`shenandoah` is accepted by the writer and understood by nobody; `g1gc` is
documented as the pre-Auto implicit default and maps to Auto, not to HighEfficiency).

The obvious fix is one canonical `GcSelection` enum with a single `FromStr`-ish normalizer used by
all three sites, accepting the legacy aliases and rejecting the rest. **My questions:** what
should `g1gc` normalize to — Auto (matching today's read paths, preserving behaviour for anyone
whose row says `g1gc`) or HighEfficiency (matching what a user reading "G1GC" would expect, but
silently changing the launch flags of existing instances on upgrade)? Should `shenandoah` be
dropped, or is accepting-and-ignoring an unknown value the safer migration? And is a persisted-row
data migration warranted, or is normalizing on read enough?

---

# What I want from you

For each of R4–R8: the fix you'd actually ship, the specific thing you'd test to be convinced it
works (a test that catches a *regression*, not just the reported bug), and anything in the pasted
code that is worse than what the review found. If my framing of any finding is wrong, or if my
proposed direction is wrong, say so plainly. I plan to do R5 and R8 first (bounded), then R6, then
R7, then R4 — tell me if that order is wrong.
