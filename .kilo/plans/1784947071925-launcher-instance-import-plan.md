# V1 Launcher Instance Migration Plan

> Historical implementation plan. Core launcher-import discovery, preview, copy, provenance, and UI work was implemented by 2026-08-05. This file is retained for decision rationale and is not current player documentation; verify remaining edge cases against source and tests.

## Goal

Add a browser-style **Import from another launcher** flow that detects Prism Launcher, CurseForge, and Modrinth App installations, previews their instances, and copies selected instances into Agora without modifying or depending on the source launcher. A successful import must produce an Agora-owned, independently launchable instance; unsupported instances are skipped before mutation with a precise reason.

## Product Decisions

- Copy all game-owned instance data; never move, hard-link, or symlink source content.
- Never import accounts, credentials, telemetry IDs, environment commands, wrapper commands, or pre/post-launch hooks.
- Detect all instances but select none by default; provide per-launcher Select all.
- Preserve compatible memory/JVM settings after a review step. Use Agora-managed Java when the old Java path belongs to a launcher; retain only existing system Java paths outside the source launcher.
- Handle each selected instance atomically. A failed/skipped instance does not roll back successful siblings.
- Use Agora's existing Vanilla/Fabric/Quilt/Forge/NeoForge support for normal tuples.
- For an unusual version of a recognized loader, allow a lightweight delegated-launch fallback only when a standard Mojang-compatible version profile can be copied and validated without translating components or reconstructing a local-only library graph. Otherwise skip the instance.
- Never substitute a nearby Minecraft or loader version.
- Re-import updates the prior Agora copy only when its meaningful payload and imported launch settings still match the last-import baseline. If the Agora copy changed, offer a new separately named copy instead; do not merge or overwrite it.
- Put the same import flow in onboarding as a skippable final step and beside Create Instance on My Instances.

## Current Gaps To Correct

- `crates/agora-core/src/import.rs` treats arbitrary directories generically, loses Minecraft/loader metadata, and writes empty content arrays.
- Prism's game root is `minecraft/` or legacy `.minecraft/`; the current directory importer copies the outer Prism control folder, while the ZIP importer skips `minecraft/` entries.
- `ImportService` promotes the filesystem directory before loader preparation, health validation, and DB registration. Cleanup usually works, but a process crash can still leave an orphan and preparation is not truly pre-promotion.
- `detect_launchers` returns only launcher-level counts, ignores launcher-configured custom roots, and uses stale/default paths for CurseForge and Modrinth.
- Current Modrinth App metadata is authoritative in read-only `app.db` tables (`instances`, `instance_content_sets`, `instance_files`, `instance_content_entries`, `instance_launch_overrides`); legacy versions used `profiles/*/profile.json`.
- Agora has no import provenance/baseline, no safe repeat-import semantics, and no per-instance launch-mode override for delegated-only imports.
- The Instance Editor file picker is not a discovery, selection, preflight, batch progress, or result experience.

## Supported Source Contract

| Source | Detection and metadata | Game payload root | Loader/settings behavior |
|---|---|---|---|
| Prism | Resolve normal OS roots plus portable/custom roots. Parse `prismlauncher.cfg` `InstanceDir`, then require parseable `instance.cfg` and `mmc-pack.json`. | Prefer `<instance>/minecraft`; use `<instance>/.minecraft` only when `minecraft` is absent. | Parse Minecraft and one supported loader component, name/icon, RAM, Java path, and JVM args. Reject enabled custom launch components that require Prism's component engine. |
| CurseForge | On Windows parse the nested `minecraft-settings` JSON string in `%APPDATA%/CurseForge/storage.json` and its `minecraftRoot`; also probe documented defaults and manual roots. Require parseable `minecraftinstance.json`. | `<minecraftRoot>/Instances/<folder>` itself. | Parse `gameVersion`, `baseModLoader` including NeoForge, pack/name/icon metadata, and applicable global/per-profile memory/JVM settings. Never query CurseForge or copy `.curseclient`/account state. |
| Modrinth | Probe current `ModrinthApp/app.db`, configured `settings.custom_dir`, Flatpak/platform variants, and legacy `com.modrinth.theseus`. Inspect schema before querying and open SQLite read-only without migrations. Fall back to legacy `profile.json` when present. | `<config_dir>/profiles/<instances.path>` for current DB rows; legacy profile directory for JSON rows. | Read applied content set, local project/version IDs, icon, install stage, and JSON launch overrides. Local import works with Modrinth integration disabled; online hash enrichment remains separately consent/network gated. |

All adapters must accept a manually selected launcher root, validate it by format rather than folder name, and return no false-positive instance directories. Detection performs no network requests and no source writes.

## Data And API Contracts

Add UI-agnostic core models for:

- `LauncherKind`: `Prism`, `CurseForge`, `Modrinth`.
- `DetectedLauncher`: stable installation key, kind, display name, canonical launcher/config root, discovered instance count, and detection warnings.
- `ImportCandidate`: stable source instance key, display metadata, canonical payload root, Minecraft/loader tuple, content counts, byte estimate, last-played time, icon preview, imported/update state, launch strategy, sanitized launch-settings preview, warnings, and `Ready`/`NeedsReview`/`Unsupported` status.
- `ImportBatchPlan`: fingerprinted selected candidates, destination IDs/names, new-vs-update action, file/byte/disk plan, launch preparation, settings decisions, conflicts, and blockers.
- `ImportBatchResult`: per-instance `Imported`, `Updated`, `Skipped`, `Failed`, or `Cancelled` outcomes plus instance ID, warnings, health result, and suggested action.

Expose one canonical path through `ImportService`:

1. `discover_launcher_instances(custom_root)` returns launchers and candidates.
2. `plan_launcher_import(selections)` re-reads source metadata, resolves collisions/provenance/update eligibility, computes disk needs, and returns an immutable fingerprinted review plan.
3. `execute_launcher_import(plan, sink, cancel)` validates the plan/source again and executes independent per-instance jobs.
4. `check_launcher_import_updates(instance_ids)` reports source missing/unchanged/updated/target modified states without mutating.

Tauri and CLI remain thin adapters. Never put Tauri types in `agora-core`. Frontend commands select candidates by backend-issued source keys; execution must canonicalize and validate every source again rather than trusting a raw frontend path.

## Implementation Phases

### 1. Land Prerequisites Before Source Adapters

1. Raise the local-state schema version and add nullable/default-safe columns to `user_instances`: `icon_path` and `launch_mode_override` (`auto`, `direct`, `delegated`; existing rows default to `auto`).
2. Make launch resolution honor `launch_mode_override` in both frontend routing and backend commands. A forced-delegated instance must reject direct-launch bypass and explain why; normal instances continue to follow the global setting.
3. Add `instance_imports` keyed by Agora instance ID with launcher kind, stable source instance key, canonical local source locator, source metadata JSON, imported settings baseline, source fingerprint, timestamps, and last result.
4. Add `instance_import_files` keyed by Agora instance ID + normalized relative path with SHA-256, size, and source metadata needed for unchanged/update checks.
5. Add a small persistent `instance_import_jobs` journal with job ID, instance/source key, staging/final paths, state, plan fingerprint, and timestamps so startup can finish or roll back a crash interrupted between rename and DB commit.
6. Cascade import rows/baselines on delete; retain them across display-name rename; do not copy provenance when cloning; never include source paths/provenance in exports, lockfiles, MCP responses, telemetry, or `instance_manifest.json`.
7. Add a validated local-icon loader (bounded PNG/JPEG/WebP size and MIME, returned as an escaped data URL or other narrowly scoped backend response) instead of broadening Tauri filesystem capabilities.
8. Generalize snapshot creation to accept an explicit normalized file set. Import updates must snapshot every changed/deleted imported payload file, not only the current fixed `TRACKED_ENTRIES`; exclude `.agora*`, staging, snapshots, and provenance internals.

### 2. Refactor The Canonical Import Pipeline

1. Split source parsing/copy policy from orchestration while retaining `import.rs`/`import_service.rs` as the public core boundary. Reuse one target allocator, path validator, content inventory builder, settings sanitizer, and promotion mechanism for `.mrpack`, Prism ZIP, generic directory, and launcher migrations.
2. Change low-level importers to prepare a staging result rather than rename into `instances/` themselves.
3. Run source validation, full metadata parsing, destination locking, copy/download, hash verification, canonical manifest construction, Minecraft metadata preparation, loader preparation, and health validation before promotion.
4. Register a persistent job before staging. At promotion, use a local-state transaction plus same-volume directory rename and job-state transitions. Startup recovery must handle every boundary: staging only, final directory with uncommitted instance row, committed row with missing final directory, and completed stale journal.
5. Roll back staging, DB rows, and any just-created official-launcher profile on required failure. Keep valid shared Agora runtime artifacts. Never delete or edit source files.
6. Stream large files with bounded buffers while calculating SHA-256 and emitting byte/file progress. Check cancellation between chunks/files; atomic rename/DB commit sections are non-interruptible.
7. Replace the fixed 500 MB disk check with plan-based available-space validation for copied bytes, staging, loader/runtime needs, and update snapshot/delta overhead plus headroom. Recheck immediately before execution.
8. Do not create a duplicate ZIP snapshot for a fresh launcher migration: the source remains untouched and `instance_import_files` is its initial baseline. Keep existing initial-snapshot behavior for ephemeral archive imports unless separately changed.

### 3. Implement Read-Only Source Adapters

1. Implement Prism detection including configured `InstanceDir`, portable roots, legacy `.minecraft`, nested-root ZIP exports, case-insensitive INI keys, managed-pack metadata, and icon lookup. Resolve only one active supported loader and reject custom enabled launch patches/components.
2. Implement CurseForge detection from `storage.json` custom root plus fallback paths. Parse current and tolerant older `minecraftinstance.json` shapes using optional fields rather than a brittle exact schema. Recognize Vanilla, Fabric, Forge, and NeoForge naming patterns with fixture coverage from real shapes.
3. Implement Modrinth current-DB scanning with `rusqlite` read-only/query-only connections, table/column capability checks, short busy timeout, parameterized queries, custom config root resolution, current content IDs/files, and launch overrides. Never run Modrinth migrations or query auth tables. Add the legacy `profile.json` adapter.
4. Treat incomplete/installing source profiles as unsupported until the source launcher finishes. Surface locked/busy/currently running source games and require closure rather than copying a live world.
5. Use `sysinfo` only as an advisory process check; correctness comes from source metadata/file stability checks. If a source file's size/mtime changes while copied, or the source inventory changes before promotion, fail that one job with `ERR_IMPORT_SOURCE_CHANGED`.
6. Keep known OS path candidates in adapter-specific functions and test Windows, macOS, Linux, portable, Flatpak, and custom-root cases without assuming that a launcher is installed.

### 4. Build Fidelity, Readiness, And Safety Planning

1. Copy every regular game-owned file below the adapter's payload root, including mods, disabled mods, configs, KubeJS/CraftTweaker scripts, options, servers, resource/shader packs, screenshots, saves/worlds, datapacks, maps, and unknown pack-specific data.
2. Never traverse symlinks, junctions, reparse points, devices, or paths outside the canonical payload root. Exclude source control files such as Prism outer metadata, CurseForge `.curseclient`/`minecraftinstance.json`, Modrinth `profile.json`, launcher credentials, and any source `instance_manifest.json`/`.agora*` data. Shared launcher runtime/cache trees outside the adapter's payload root are never copied; do not drop an in-root file merely because a generic folder name resembles a cache.
3. Build all canonical `InstanceManifest` arrays from staged content. Hash every artifact; parse JAR IDs, versions, dependencies, aliases, and enabled state with `jar_metadata`; inventory zipped and directory resource packs/shaders/datapacks/worlds; retain unknown files on disk even when they are not managed artifacts.
4. Resolve identities in this order: trusted source metadata, exact local registry SHA-256 match, optional consented Modrinth hash lookup, then stable manual/imported identity. Preserve Modrinth IDs from local Modrinth metadata without requiring live API access. Never add CurseForge egress.
5. Normalize source launch settings into Agora fields. Extract RAM and known GC flags, remove duplicate `-Xms/-Xmx`, reject classpath/jar/agent/native-path/credential-bearing arguments, list every omitted argument in review, and never carry shell hooks, wrappers, or environment commands.
6. For cataloged loader tuples, use `LoaderService` and allow normal direct/delegated behavior.
7. For a recognized but uncataloged loader version, probe only for a standard Mojang version JSON whose ID/inheritance/main class match the detected tuple and whose libraries use safe relative Maven paths with usable URLs/hashes. Copy the minimal version profile atomically into the official `.minecraft` without overwriting a conflicting profile, set `launch_mode_override=delegated`, and verify delegated preparation. Do not translate Prism components or copy a local-only library graph; skip when the lightweight path is insufficient.
8. Run health validation after the staged manifest is complete. Health warnings/blockers are reported and shown after import but do not silently remove, replace, enable, or disable source content.
9. Return unsupported reasons during planning for missing game version, ambiguous/multiple loaders, unsupported custom components, invalid source metadata, unavailable required network policy, missing Mojang launcher for delegated fallback, unsafe paths, and insufficient disk.

### 5. Add Safe Repeat-Import Updates

1. On discovery, match prior imports by launcher installation/source key, not display name. Show `Already imported`, `Source updated`, `Source missing`, or `Agora copy modified`.
2. Define unchanged Agora state as matching baseline hashes for every imported meaningful file plus the normalized manifest tuple and imported launch settings. Ignore Agora internals, snapshots, launch history, `logs/`, and `crash-reports/`; treat changes/additions to mods, config, options, screenshots, worlds/saves, scripts, packs, or other user payload as modified.
3. If both source and Agora match the baseline, report unchanged and perform no work.
4. If source changed and Agora is unchanged, create a delta plan: additions, replacements, deletions, metadata/settings changes, loader preparation, affected snapshot bytes, and disk requirement.
5. Snapshot only affected existing imported files, stage and verify all additions/replacements, apply the delta through atomic file actions, rebuild the manifest/baseline, run health validation, and commit provenance last. Restore the snapshot and old DB state on failure.
6. If Agora changed, disable Update and offer `Import as new copy` with an editable collision-safe destination name. Do not add three-way merge or source-authoritative overwrite in V1.
7. Preserve the Agora display name, lock state, snapshots, and launch history during a safe update. Treat source version/loader/settings changes as reviewable plan changes and keep forced delegation only while required.

### 6. Build The Browser-Style UI

1. Add a reusable responsive import wizard, opened from onboarding and My Instances, with stages: Detect, Select, Review, Importing, Results.
2. Detect locally on entry and group candidates under Prism, CurseForge, and Modrinth cards. Provide search, expand/collapse, per-instance checkboxes, per-launcher Select all, Clear all, refresh, and Add launcher location via a directory picker. Select none initially.
3. Candidate rows show icon, name, source, Minecraft/loader, content counts, approximate size, last played, direct/delegated strategy, already-imported/update state, and concise warnings/unsupported reasons.
4. Review shows destination names, new/update action, total copy and peak disk bytes, settings to preserve/omit, loader strategy, network prerequisites, source-running warnings, collisions, and skipped selections. The Import button is the single approval for all ready candidates.
5. Listen to one batch operation plus per-instance progress events. Show current phase/file/bytes and completed/failed counts. Cancellation rolls back the current instance, keeps already completed instances, and cancels remaining work.
6. Results list imported/updated/skipped/failed instances with actionable reasons and buttons to open successful instances. Refresh My Instances immediately.
7. Add the wizard as a skippable final onboarding step after registry setup so service/network choices already exist. Persist only the normal onboarding completion state; skipping import must not suppress the permanent My Instances entry point.
8. Replace the Instance Editor's misleading new-instance ZIP import section with a link to the shared launcher/file import flow, while retaining pack import/export actions in their appropriate context.
9. Display source badges and forced-delegated status on imported instance cards without exposing absolute source paths outside the import/update details view.

### 7. Adapter Boundaries, Errors, And Rollout

1. Add structured errors and suggested actions for source busy/changed, unknown source schema, unsupported loader/components, destination changed, disk full, unsafe path, transaction recovery, and delegated profile conflict.
2. Keep local Modrinth import separate from `modrinth_enabled` consent and `network_modrinth_enabled` egress permission. Only optional online enrichment requires both and no lockdown.
3. Keep the existing file/URL import commands working while routing them through the corrected canonical staging/promotion pipeline. Existing instances imported before provenance exists remain ordinary instances and are not guessed/relinked automatically.
4. Add thin CLI commands for detection, plan JSON, execution, and update check, or at minimum expose equivalent core APIs and preserve current `agora import <path>` behavior. CLI and desktop must not implement separate parsing/copy logic.
5. Update the in-app Guide and BACKLOG acceptance records only after the implementation and tests land. Document exclusions and the delegated-only unusual-loader fallback clearly.
6. Do not add broad Tauri filesystem, shell, SQL execute, or network permissions. Backend commands perform scoped reads/copies; local icons use the validated loader; no new external domains are required.

## Validation Plan

### Core Fixtures And Unit Tests

- Prism fixtures: normal/custom `InstanceDir`, `minecraft` and `.minecraft`, nested ZIP root, each supported loader, disabled JAR, managed pack IDs, custom icon, JVM settings, unknown enabled component, malformed INI/JSON, and symlink/reparse escape.
- CurseForge fixtures: custom `minecraftRoot`, current/older metadata, Vanilla/Fabric/Forge/NeoForge, global and per-profile memory, missing/in-progress profile, `.curseclient`, and malformed nested settings JSON.
- Modrinth fixtures: current `app.db` schema with custom dir/content/launch overrides, WAL/read-only access, missing/newer optional columns, locked DB, legacy `profile.json`, local Modrinth IDs, and auth tables that tests prove are never queried.
- Content tests: every manifest array, directory and ZIP packs, unknown files retained, disabled mods, JAR dependency metadata, source/registry identity precedence, no online enrichment when consent is off, and no CurseForge requests.
- Settings tests: RAM/GC extraction, allowed flags, rejected agents/classpaths/hooks/env, launcher-owned Java replacement, and existing system Java retention.
- Loader tests: cataloged direct readiness, normal delegated mode, accepted minimal odd-version profile, profile collision, unsafe/local-only libraries, custom Prism component skip, and no version substitution.
- Provenance tests: unchanged dedupe, source-only update, source missing, logs-only Agora change allowed, meaningful Agora change blocked, fresh-copy fallback, rename/delete/clone/export behavior, and absolute paths never serialized into exportable artifacts.
- Security tests: source tree byte-for-byte unchanged, recursive-source rejection, path traversal, reserved names, symlink/junction/reparse rejection, management-file exclusion, bounded icon decoding, source mutation mid-copy, and no credentials copied/logged.
- Transaction fault-injection tests at staging, loader preparation, snapshot, DB insert, rename, commit, health scan, cancellation, and startup recovery boundaries; assert no unregistered live instance and no lost prior instance.
- Disk/progress tests with sparse/large files, zero-byte files, partial batch failure, cancellation during a streamed file, and exact cleanup.

### Integration And UI Tests

- Import one fixture from each launcher through `ImportService`; assert source unchanged, complete manifest, DB row/provenance, loader readiness, health report, and listing through `InstanceService`.
- Prove a cataloged imported instance reaches the direct launch planner and an unusual accepted profile routes only through delegated launch.
- Prove batch success is per-instance atomic and repeated execution is idempotent.
- Playwright: onboarding import detection/skip, selection-none default, search/select-all, custom location, review/settings warnings, progress/cancel, mixed result summary, destination collision, already-imported/update states, modified-target fresh-copy offer, and My Instances refresh.
- Playwright: imported icon/source badge, forced delegated launch despite global direct mode, keyboard/focus behavior, narrow-window layout, and escaped untrusted names/errors.
- Run `/desktop`, `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, and the desktop Playwright suite. Preserve existing `.mrpack`, manual file import, create, clone, export, snapshot, update, direct launch, and delegated launch tests.

## Explicitly Out Of Scope For V1

- Importing Microsoft/Modrinth/CurseForge accounts, credentials, social data, telemetry, or launcher-global preferences unrelated to instance launch readiness.
- Moving/deleting source instances or uninstalling the old launcher.
- Background synchronization, automatic source polling, three-way merging, or overwriting an Agora-modified copy.
- Calling CurseForge APIs or recreating CurseForge-managed update behavior.
- Executing arbitrary Prism components, custom loader patches, shell hooks, wrappers, or environment commands.
- Supporting launchers other than Prism, CurseForge, and Modrinth in this UI; the adapter interface may enable later additions.

## Research Anchors

- Agora foundation: `crates/agora-core/src/import.rs`, `import_service.rs`, `instance_service.rs`, `models.rs`, `db.rs`, `jar_metadata.rs`, `snapshot.rs`; `desktop/src/pages/Instances.tsx`, `InstanceEditor.tsx`, `Onboarding.tsx`.
- Prism format behavior: `PrismLauncher-develop/launcher/InstanceList.cpp`, `InstanceImportTask.cpp`, and `minecraft/MinecraftInstance.cpp` show `instance.cfg`, `mmc-pack.json`, configured instance roots, and `minecraft`/`.minecraft` game roots.
- Modrinth behavior: `packages/app-lib/src/api/pack/import/{mod.rs,mmc.rs,curseforge.rs}`, `state/dirs.rs`, `state/db.rs`, current instance/content migrations, and the shared import-stage UI establish current path, DB, and batch-selection behavior.
- Local CurseForge installation confirms `storage.json` contains a nested `minecraft-settings.minecraftRoot` and instances contain current `minecraftinstance.json` metadata.
