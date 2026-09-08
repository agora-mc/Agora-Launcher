# Task: reject modpack updates that require a different Minecraft/loader runtime

Repo: Agora, a Minecraft mod launcher. Rust workspace; `crates/agora-core` is the shared library.
You are in a git worktree on branch `fix/pack-update-runtime-guard`.

## The defect

Applying a `.mrpack` update over an existing instance replaces every mod file but never looks at
the pack's declared runtime requirements, and never updates the instance's own runtime settings.

The `.mrpack` `modrinth.index.json` format carries a `dependencies` object, e.g.

```json
{ "dependencies": { "minecraft": "1.21.1", "neoforge": "21.1.66" } }
```

with keys `minecraft`, `fabric-loader`, `forge`, `neoforge`, `quilt-loader`. The parser in
`crates/agora-core/src/pack_update.rs` (around line 125) omits the field entirely:

```rust
#[derive(Deserialize)]
struct MrpackIndex {
    #[serde(default)] name: String,
    #[serde(default, alias = "versionId")] version_id: Option<String>,
    #[serde(default)] files: Vec<MrpackFile>,
    #[serde(default)] overrides: String,
    // no `dependencies`
}
```

`update_pack` (around line 1570) then previews, stages, reconciles the manifest, and applies the
merge — without ever reading or writing `minecraft_version`, `loader`, or `loader_version`. So an
instance pinned to 1.20.1 + Fabric can be updated to a pack built for 1.21.1 + NeoForge: the mods
all change, the runtime does not, and the next launch runs 1.21 mods on a 1.20.1 client with the
wrong loader.

## Scope of THIS task: the rejection guard only

Full runtime migration (installing the new loader, changing the Minecraft version, and committing
all of that atomically with the file changes) is a separate project and is **not** in scope. Do
not attempt it. Do not call into `version_migration.rs` or `loader_service.rs`.

What this task delivers: **parse the requirements, and reject any update that would require a
runtime change, before anything is staged or mutated.**

## Required behaviour

1. Add `dependencies: BTreeMap<String, String>` (`#[serde(default)]`) to `MrpackIndex` and surface
   the parsed requirement as a typed value — the required Minecraft version, and the required
   loader family plus loader version where present.

2. Distinguish "the pack declares no requirements" from "the pack declares requirements that
   match". Never infer "keep the existing loader" merely because parsing found nothing. A pack
   whose `dependencies` is absent or empty, or that names a loader family this launcher does not
   know, is **unsupported** and must be rejected — not silently treated as compatible.

3. Validate during `preview_pack_update`, and again in `update_pack` **before** `stage_pack` runs,
   so nothing is downloaded or written for an update that will be refused. Compare against the
   instance's current `minecraft_version`, `loader`, and `loader_version`.

4. Reject when *any* of those three differ — including a loader-version-only change, not just a
   Minecraft-version or loader-family change. Return a `PackUpdateOutcome::Failed` with a distinct
   phase (e.g. `"runtime"`) whose message states the current tuple and the required tuple, so the
   user can see exactly what the pack wants. The existing `PackUpdateOutcome::Failed` has
   `rolled_back: bool` — set it `false`, because nothing was applied.

   Do **not** tell the user to "migrate first, then update": changing the runtime while keeping the
   old pack's mods produces the same incompatibility in the other order. Just state the mismatch.

5. Surface the requirement in the preview result too, so the UI can show it before the user
   commits. Add a field to the preview struct; do not remove or rename existing fields.

6. While you are in the parser: `files` is `#[serde(default)]`, so a malformed index with no
   `files` key currently deserialises to an empty vector. Check whether the downstream merge treats
   an empty `theirs` as "delete everything". If it does, distinguish an absent `files` key from an
   explicitly empty list and reject the absent case. If it does not, say so in your report and
   leave it alone — do not guess.

## Files you may change

- `crates/agora-core/src/pack_update.rs` — the main job.
- `crates/agora-core/src/pack_merge.rs` — only if step 6 requires it.

Do not touch anything else. Do not change the signature of `pub fn update_pack` or
`pub fn preview_pack_update`. Do not touch `desktop/`. If a Tauri command or CLI call site fails to
compile because you added a field to the preview struct, fix that call site minimally and say so.

## Tests you must add

In the existing `#[cfg(test)] mod tests` of `pack_update.rs`. Each must fail before the change.
There are existing helpers in that module for building a test `.mrpack` and a test instance — reuse
them rather than inventing new fixtures.

1. **Matching runtime still applies.** A pack whose `dependencies` exactly match the instance
   updates successfully, as today. This is the regression guard: the fix must not break the normal
   case.
2. **Minecraft version change is rejected.** Same loader, different `minecraft` → `Failed` with
   phase `"runtime"`, `rolled_back: false`.
3. **Loader family change is rejected.** `fabric-loader` → `neoforge`.
4. **Loader version-only change is rejected.** Same family, different version.
5. **Missing/empty `dependencies` is rejected**, and an unknown loader family key is rejected.
6. **Nothing is mutated or downloaded on rejection.** For each rejection case assert: the instance
   directory contents are byte-identical to before, `instance_manifest.json` is unchanged, the
   database runtime fields are unchanged, and the staging directory was not populated. Run this
   with `skip_health_scan = true` as well as `false`.
7. **The preview reports the requirement** without applying anything.

## Verify before you finish

```
cargo fmt --all --check
cargo clippy -p agora-core --all-targets --all-features -- -D warnings
cargo test -p agora-core --lib pack_update
```

Clippy runs with `-D warnings`. Report anything you could not make pass, and flag anything you
found that looks worse than what is described here.
