# Health, Crash Doctor, Auth, Memory, and Launch Plan

> Historical implementation plan. The v1 health, Crash Doctor, authentication, memory, and launch work described here was substantially implemented by 2026-08-05. This file is retained for decision rationale and is not current product documentation; verify behavior against source, tests, and the in-app guide.

## Goal

Deliver a local-first v1 diagnostic and launch overhaul that removes avoidable launch friction, makes Crash Doctor useful without copy/paste, preserves safe recovery, refreshes account sessions reliably, recommends memory without overriding users, and improves measured launch performance.

## Confirmed Decisions

- Keep running the local health scan before launch. Skip only the dialog when there are no active blockers or unmuted warnings.
- Missing optional dependencies become recommendations. Recommendation-only reports never interrupt launch.
- Recommendations are collapsed when shown alongside real findings and can be muted as one group.
- Use a validated health scan token to avoid the second JAR parse. Do not add a larger secret-bearing prepare/resume launch session for v1.
- Keep Crash Doctor local-first. Do not build GitHub issue search, submission, or curated issue governance for v1.
- Add MCP skill guidance for optional upstream issue research by an AI agent after local evidence is exhausted.
- Existing instances remain in Manual memory mode because their current 4 GB value does not reveal user intent. New instances default to Auto.
- Preserve full snapshot coverage, including saves. Optimize incremental comparison and hashing rather than weakening recovery.
- AI remains optional supplementary explanation. Deterministic evidence and recovery must work offline.
- Do not upload crash evidence or expose arbitrary filesystem reads through Tauri IPC.

## Confirmed Root Causes

| Area | Current problem |
|---|---|
| Health dialog | `useProcessController.startLaunch` opens on raw warnings before `HealthDialog` loads persisted mutes. |
| Auth visibility | `approveLaunch` leaves `healthReport` set when `ERR_MSA_AUTH_REQUIRED` arrives, so the Health dialog remains above the instance error. |
| Health noise | `MissingOptionalDependency` is mixed into warnings and can generate a large flat list. |
| Duplicate work | Desktop calls `check_instance_health`, then a healthy core launch parses every enabled JAR again. |
| Crash discovery | The card Troubleshoot action deliberately opens `PasteLogModal` instead of attempting automatic evidence discovery. |
| Missed direct crashes | Direct launch writes `last_launched_at` after process exit, which can make the crash report older than the comparison timestamp. |
| Disconnected diagnosis | Curated regex triage and action buttons are separate from the signal-based investigation path and never appear in Crash Doctor. |
| Weak evidence input | Investigation reads one crash report only; it ignores `latest.log`, `debug.log`, fatal JVM logs, and additional selected files. |
| Premature confirmation | Crash Doctor asks whether a disable fixed the crash as soon as Java starts, not after the correlated launch outcome. |
| Scoring cost/noise | Suspect scoring performs pairwise DB calls and can return every installed mod with zero evidence. |
| MSA architecture | `LaunchService::load_inputs` requires and refreshes MSA credentials even for delegated launch. MSA refresh has no single-flight and does not classify `invalid_grant`. |
| GitHub expiry race | Concurrent post-401 handlers can rotate twice because they do not check whether the failed token was already replaced while waiting. Some network paths use raw `get_token`, and successful refresh ignores durable-store errors. |
| Warm launch cost | Java discovery is repeated, library/assets materialization is sequential, health is duplicated, and snapshot comparison hashes broad content repeatedly. |

## Implementation Sequence

### 1. Establish Baselines and Characterization Tests

Run the existing CLI integration suite and targeted desktop launch/Crash Doctor E2E suites before production edits. Record command duration, test count, failures, and warm launch behavior in the implementation session notes rather than committing generated reports.

Add failing characterization tests before changing each subsystem. Cover muted-health prompting, MSA-after-health behavior, direct crash timestamp semantics, crash evidence selection, GitHub concurrent 401 refresh, and MSA permanent versus transient refresh failures.

Add lightweight phase timing to `agora-core` before performance optimization so changes are driven by measurements. Extend `LaunchProgress` with a defaulted structured phase-complete callback to avoid breaking hosts, and collect durations/cache hit counts for input loading, auth, health, Java discovery, resolve, materialization, command build, snapshot, and spawn.

Expose timings through `agora launch --timings` and existing Tauri `launch-progress` events. Keep normal UI output concise and keep JSON stdout machine-pure.

Primary files:

- `crates/agora-core/src/launch_service.rs`
- `crates/agora-core/src/launch_planner.rs`
- `crates/agora/src/main.rs`
- `crates/agora/tests/cli_integration.rs`
- `desktop/src-tauri/src/commands.rs`
- `desktop/src/lib/useProcessController.ts`

### 2. Separate Health Recommendations and Centralize Preferences

Add a typed `Recommendation`/`RecommendationKind` collection to `HealthReport`. Move only `MissingOptionalDependency` out of `WarningKind`; inventory failures, drift, duplicate IDs, unknown files, and incompatibilities remain warnings or blockers.

Compute health score from blockers and warnings only. A report containing recommendations alone is green and never opens the launch dialog.

Replace per-render N-setting reads with one structured `health_preferences` value containing muted warning keys and `mute_all_recommendations`. Add a core settings helper that imports currently active legacy `health_silenced_*` values on first use so existing mutes continue working.

Give findings stable semantic keys. Do not key all global findings only as `kind/global`; include the relevant semantic identities so muting one conflict cannot accidentally mute unrelated future conflicts.

Load preferences in `startLaunch` before deciding whether to present Health Check. On preference read failure, show real warnings rather than silently suppressing them. Blockers are never mutable.

Render recommendations in a closed Radix disclosure inside `HealthDialog` only when the dialog is already open for warnings/blockers. Show count, explicit low-severity wording, and one group mute switch. Keep individual warning mutes.

Primary files:

- `crates/agora-core/src/health.rs`
- `crates/agora-core/src/settings.rs`
- `desktop/src/lib/tauri.ts`
- `desktop/src/lib/useProcessController.ts`
- `desktop/src/components/HealthDialog.tsx`
- `desktop/e2e/health-launch.spec.ts`

### 3. Reuse a Validated Health Scan and Fix Error Ownership

Introduce a core `HealthService` around the current pure scanner. It returns the report plus a `scan_token` derived from the manifest bytes, enabled mod directory entries (name, size, modification time), and registry identity relevant to aliases/conflicts.

Cache the report in the process-local `CoreContext`. Desktop launch commands accept the scan token. `LaunchService` recomputes the cheap input fingerprint and reuses the report only on an exact match; a stale/missing token forces a full rescan. Explicit warning/blocker approval applies only to the matching scan token.

Add a per-JAR parsed metadata cache keyed by a known manifest SHA-256 when available, otherwise an observed SHA-256 plus file metadata. Changed or unknown files are reparsed. Cache corruption degrades to a fresh parse.

Carry `errorCode` in `ProcessState`. If health-approved launch returns `ERR_MSA_AUTH_REQUIRED` or the new MSA-expired equivalent, clear `healthReport`, preserve the structured launch error, and show an Accounts/Sign In action on the instance surface. Other launch failures remain recoverable in Health Check as today.

Return a structured launch start result containing `session_id`, mode, and optional PID from direct and delegated commands. This supports later Crash Doctor outcome correlation and removes PID-only assumptions.

Primary files:

- `crates/agora-core/src/ctx.rs`
- `crates/agora-core/src/health.rs`
- `crates/agora-core/src/launch_service.rs`
- `desktop/src-tauri/src/commands.rs`
- `desktop/src/lib/tauri.ts`
- `desktop/src/lib/useProcessController.ts`
- `desktop/src/pages/Instances.tsx`
- `desktop/src/App.tsx`

### 4. Build a Unified, Safe Crash Evidence Collector

Add `CrashEvidenceService` in `agora-core` and make it the input to GUI, CLI, and MCP diagnostics. Retain `CrashService` for safe mutations, telemetry, and scoring rather than duplicating it.

Automatic sources, newest first:

| Source | Selection rule |
|---|---|
| `crash-reports/*.txt` | Prefer the newest report; timestamps are a hint, not a hard exclusion. |
| `logs/latest.log` | Always consider when it exists and include when contemporaneous with the primary failure. |
| `logs/debug.log` | Include when its mtime is near the selected failure and it contributes loader detail. |
| `hs_err_pid*.log` | Include the newest fatal JVM report in the instance root. |
| Process capture | Include the bounded, token-redacted direct-launch buffer for the correlated session when available. |

Choose a coherent evidence set around the primary source (same launch window) rather than concatenating every historical file. Mark stale, supplementary, truncated, and user-added evidence explicitly.

Bound reads by source and aggregate size. Read a useful head and tail from oversized text so startup metadata and the root cause are both retained. Decode invalid UTF-8 lossily, strip NUL/control noise, preserve line numbers, and never log evidence text.

Do not expose a generic `read_file(path)` command. Add a desktop-only command that opens a native multi-file picker and returns bounded text attachments directly, with basename, size, truncation, and source type but no full path. Accept diagnostic text extensions only. The backend owns picker and read in one command so a forged frontend invocation cannot read arbitrary paths.

Keep paste as a secondary fallback under “Add evidence,” not the default entry flow.

Move `last_launched_at` updates to the successful process/handoff start boundary for both modes. Remove the direct post-exit timestamp write. Keep evidence collection tolerant of legacy bad timestamps.

Primary files:

- `crates/agora-core/src/crash_service.rs`
- `crates/agora-core/src/crash_diagnostics.rs`
- new `crates/agora-core/src/crash_evidence.rs`
- `crates/agora-core/src/launch_service.rs`
- `crates/agora-core/src/instance_service.rs`
- `desktop/src-tauri/src/commands.rs`
- `desktop/src/lib/tauri.ts`

### 5. Merge Signatures, Parsing, Health Context, and Suspect Ranking

Replace the two disconnected investigation entry points with one `investigate_evidence` result containing sources, a normalized fingerprint, exception chain, exact evidence excerpts, loader diagnostics, curated signature matches, relevant health findings, ranked suspects, confidence, and safe actions.

Run curated signatures first but continue deterministic parsing so the UI can explain why a fix applies. Replace `action_button_json` at the app boundary with a validated typed action enum: install a registry mod, open Java settings, open installed content, or focus an evidence source. Keep the compiled JSON column backward compatible and validate optional action type fields in the compiler.

Render signature solution markdown with the existing sanitized renderer and no raw HTML. Action targets are local allowlisted operations only.

Improve parsing for nested `Caused by` chains, Fabric/Quilt resolution errors, Forge/NeoForge missing dependencies, mixin class/config names, unsupported Java class versions, OOM variants, native/JVM fatal errors, and graphics initialization failures.

Refresh missing installed JAR package metadata once during investigation so older installs can receive stack attribution. Cache the result; do not issue per-mod network requests.

Batch telemetry and conflict reads. Rank enabled mods with positive evidence only, give each signal a human explanation and excerpt, and use confidence bands instead of presenting an unexplained decimal as certainty. Remove or implement documented signals consistently; the MCP skill and spec must not claim recency signal F unless it is actually computed.

Record the installed set and signature/fingerprint once per correlated crash, then learn primarily from explicit confirmed/ruled-out outcomes. Do not populate pair telemetry from a list containing every zero-score mod.

Expand `crash-signatures/` only with high-specificity, locally actionable patterns. Each addition requires positive fixture matches, realistic large-log evaluation, and close non-match tests. Initial targets are Fabric and Forge/NeoForge dependency failures, unsupported Java, JVM/native memory failure, duplicate mod IDs, and graphics initialization; avoid generic patterns that create another noisy recommendation system.

Primary files:

- `crates/agora-core/src/crash_service.rs`
- `crates/agora-core/src/crash_diagnostics.rs`
- `crates/agora-core/src/db.rs`
- `crash-signatures/*.json`
- `compiler/compile.py`
- `compiler/test_compile.py`
- `desktop/src/components/CrashInvestigator.tsx`

### 6. Replace the Crash Doctor Orchestration and UX

Keep the existing safe snapshot/restore and disable primitives, but replace the 844-line component’s monolithic state machine with a small coordinator and focused source, summary, known-fix, evidence, suspect, and experiment sections.

Opening Troubleshoot or Investigate immediately runs automatic collection and shows what was selected. Users can add multiple files or paste text, remove supplementary sources, and rerun without leaving the dialog.

Show a summary first: likely category, curated fix if present, confidence, and the exact files analyzed. Put full highlighted logs and signal details behind explicit disclosures. Highlight ERROR/WARN/INFO and the exact matched lines without using HTML injection.

Create the recovery snapshot lazily immediately before the first mutation. Read-only analysis still works when snapshot creation is unavailable; mutation remains disabled with an explanation.

Replace the loop of separate disable calls with one core atomic test-plan operation under the instance lock. It disables the selected suspect and approved dependents, rolls back on any partial failure, and returns the exact changed set.

Correlate test launches by `session_id` and wait for `game-exited` or delegated outcome instead of asking immediately after start. Behavior by outcome:

| Outcome | Crash Doctor behavior |
|---|---|
| Same fresh fingerprint while suspect is absent | Restore the pre-test state, mark the suspect ruled out, and advance. |
| Different fresh crash | Restore and show that the failure changed; do not confirm or rule out automatically. |
| Clean/stable success | Ask the user to confirm the fix before recording attribution and keeping the chosen disable set. |
| Short abandoned/cancelled/launch error | Restore and mark the test inconclusive. |
| Health or MSA decision interrupts start | Keep the experiment pending and resume only after the canonical launch controller reports a session start. |

Remove the Instances-page effect that can auto-open a modal for only one “last” instance. Instead, surface a post-crash alert/toast tied to the `game-exited` instance with an Investigate action. Manual Troubleshoot remains available at all times.

Replace `PasteLogModal` on instance cards with direct Crash Doctor launch. Add the matching Investigate button beside Open in Folder in `InstanceEditor`, wired through App-level state so navigation does not lose the investigation.

Primary files:

- `desktop/src/components/CrashInvestigator.tsx`
- new focused components under `desktop/src/components/crash-doctor/`
- `desktop/src/pages/Instances.tsx`
- `desktop/src/pages/InstanceEditor.tsx`
- `desktop/src/App.tsx`
- `desktop/src/lib/useProcessController.ts`
- `desktop/e2e/crash-investigator.spec.ts`
- `desktop/e2e/launch-events.spec.ts`

### 7. Give the CLI the Same Realistic Diagnostic Path

Change `agora crash investigate <instance>` to use the unified evidence collector and full investigation result, not only `suggest_mod_incompatibility` on the newest crash report.

Support repeatable `--file <path>` arguments for explicitly selected CLI evidence. CLI paths are explicit user inputs, canonicalized and bounded by the same text/size policy. Add `--paste` only if stdin is a TTY-safe explicit mode; never block implicitly waiting for input.

Human output shows source selection, signature/summary, confidence-ranked suspects, evidence excerpts, and next safe actions. JSON output exposes the stable typed result and keeps diagnostics/progress off stdout.

Do not add unattended disable/relaunch automation to the CLI in this scope. Core mutation tests and desktop guided tests cover the reversible experiment.

Create synthetic but realistic, privacy-free fixtures through test builders:

| Fixture | Assertion |
|---|---|
| Fabric missing dependency crash report | Curated dependency diagnosis and install action. |
| Mixin crash with one mapped package | Correct mod ranks above unrelated installed mods with cited frame. |
| Forge/NeoForge nested `NoClassDefFoundError` | Deep cause and owning package are extracted. |
| OOM in `latest.log` with no crash report | `latest.log` is selected and memory action appears. |
| `hs_err_pid` fatal JVM log | Fatal JVM category is detected. |
| Old report plus fresh `latest.log` | Collector selects the coherent fresh evidence set. |
| Oversized/malformed/binary-like input | Bounded truncation and graceful parsing; no panic/OOM. |
| JSON mode | Valid stdout schema with no progress contamination. |

Primary files:

- `crates/agora/src/main.rs`
- `crates/agora/tests/cli_integration.rs`
- `crates/agora-core/src/crash_evidence.rs`
- `crates/agora-core/src/crash_service.rs`

### 8. Add Explainable Automatic Memory Mode

Increment local-state schema to v9 and add `jvm_memory_mode TEXT NOT NULL DEFAULT 'manual'`. Migration leaves every existing row Manual. New launcher-created instances default Auto. Imported instances with an explicit source-launcher memory value remain Manual; imports without a value default Auto.

Use one shared recommendation function for direct command construction, delegated launcher profiles, Instance Editor preview, and CLI diagnostics.

Build a cheap `ContentSummary` from enabled manifest entries plus filesystem metadata: enabled mod count, aggregate mod JAR bytes, resource-pack count/bytes, loader, and Minecraft version. Do not parse JARs again and do not persist file sizes in the manifest.

Use explainable tiers and the highest tier triggered by count or compressed mod bytes:

| Pack proxy | Initial heap tier |
|---|---|
| Vanilla/no enabled mods | 2 GB |
| Up to 40 mods and 256 MiB | 4 GB |
| Up to 100 mods and 768 MiB | 6 GB |
| Up to 200 mods and 1.5 GiB | 8 GB |
| Above either heavy threshold | 10 GB |
| Above 350 mods or 2 GiB | 12 GB |

Treat large resource packs as a secondary one-tier adjustment. Do not add heap merely because shaders exist; shader demand is primarily GPU/VRAM and would make the estimate misleading.

Round to 512 MiB. Clamp to 32 GB and to system headroom using both the existing 75% ceiling and at least 2 GB reserved for the OS where physically possible. Return an explicit insufficient-system-memory warning when the requested tier cannot fit instead of claiming the clamped value is ideal.

Auto recalculates when installed content changes. Manual never changes silently. The Instance Editor gets an Auto switch, effective recommendation, factor explanation, and “Use custom value” path. An OOM signature may propose the next tier, but applying it requires confirmation and changes the instance to Manual unless the user chooses to keep Auto.

Add `agora instance recommend-memory <id>` for transparent diagnostics and tests.

Primary files:

- `crates/agora-core/src/db.rs`
- `crates/agora-core/src/models.rs`
- `crates/agora-core/src/gc.rs`
- `crates/agora-core/src/installed_content.rs`
- `crates/agora-core/src/instance_service.rs`
- `crates/agora-core/src/launch_service.rs`
- `crates/agora-core/src/launcher_import_service.rs`
- `desktop/src/pages/InstanceEditor.tsx`
- `desktop/src/pages/Instances.tsx`
- `desktop/src/lib/tauri.ts`
- `crates/agora/src/main.rs`

### 9. Make GitHub and Microsoft Refresh Durable and Race-Safe

Create one GitHub authenticated-request helper used by every network caller. It obtains a fresh token, executes once, and on 401 calls refresh with the exact failed access token.

Under the refresh mutex, re-read storage. If the stored access token differs from the failed token, another caller already rotated it; return the current token without a second refresh. Otherwise refresh once, atomically persist the complete rotated bundle, then retry once. A successful refresh is not considered complete if durable secure storage fails.

Use atomic temp-write/sync/rename for encrypted fallback secrets. Preserve credentials on network, 5xx, malformed-success, or secure-store errors. Clear only for explicit permanent provider responses such as `bad_refresh_token`, `expired_token`, HTTP 400/401 with the expected OAuth error, revocation, or known refresh expiry.

Replace raw `get_token()` in network paths with the helper, including core install resolution and desktop governance diagnostic helpers. Raw token reads remain acceptable only for non-network status display/tests.

Add an injectable MSA HTTP abstraction and a single `get_valid_credentials` path with a mutex and double-check. Parse OAuth error bodies. On `invalid_grant`/revocation, clear credentials and return a structured Microsoft sign-in-required action. On transient failure, preserve stored credentials and report refresh unavailable without presenting the user as signed out.

Refresh MSA on demand within the existing five-minute margin; do not add a polling timer. Skip MSA load/refresh entirely for delegated launch. Direct launch remains the only mode that requires Minecraft credentials.

Settings status should distinguish Connected, Refresh Needed/Refreshing, Offline With Stored Session, and Sign In Required without exposing tokens.

Deterministic tests cover proactive refresh, concurrent expiry, concurrent 401, token rotation, durable-store failure, malformed/transient responses, permanent revocation, MSA `invalid_grant`, MSA network failure, and delegated launch without MSA. All tests use isolated fallback directories or injected credentials and never touch Keychain, especially on macOS.

Primary files:

- `crates/agora-core/src/auth.rs`
- `crates/agora-core/src/msa.rs`
- `crates/agora-core/src/launch_service.rs`
- `crates/agora-core/src/install_service.rs`
- `crates/agora-core/src/resolver.rs`
- `desktop/src-tauri/src/auth.rs`
- `desktop/src-tauri/src/governance.rs`
- `desktop/src-tauri/src/registry_sync.rs`
- `desktop/src/pages/Settings.tsx`
- `desktop/src/lib/tauri.ts`

### 10. Optimize Measured Launch Hotspots

Implement the correctness wins regardless of timings: one validated health parse per unchanged launch, no delegated MSA work, and no repeated Java probe when a valid app-session cache exists.

Cache Java discovery in `CoreContext` with a short TTL and explicit invalidation after runtime provisioning, Java setting changes, or a failed selected runtime. Preserve explicit per-instance Java overrides as authoritative.

Materialize independent libraries and asset objects with bounded concurrency. Keep deterministic classpath order, unique atomic destinations, host/category policy checks, redirects, hashes, native extraction limits, and first-download verification unchanged. Limit concurrency and in-flight bytes to avoid turning a cold launch into a memory spike.

Add an instance-local file fingerprint cache shared by health and snapshot comparison. Key entries by relative path, size, modification time, and known content hash. Full hashing remains mandatory for new/changed files, snapshot creation of uncached content, restore verification, and downloaded artifact verification. Corrupt caches are discarded.

For LKG comparison, read reference hashes from the immutable snapshot manifest rather than rereading every snapshot blob. Continue tracking saves and all current `TRACKED_ENTRIES`.

Audit delegated preparation after timings. Move only proven direct-only work out of delegated launch while preserving loader/profile artifacts required by the official launcher; do not assume all materialization is unnecessary.

Use repeatable synthetic small/medium/large CLI fixtures for cold and warm runs. Assert operation counts, cache reuse, single health parse, bounded concurrency, and no repeated Java process probes. Report wall-clock deltas, but do not add flaky absolute timing assertions to CI.

Primary files:

- `crates/agora-core/src/ctx.rs`
- `crates/agora-core/src/java.rs`
- `crates/agora-core/src/launch_planner.rs`
- `crates/agora-core/src/snapshot.rs`
- `crates/agora-core/src/health.rs`
- `crates/agora-core/src/launch_service.rs`
- `crates/agora-core/tests/launch_planner_integration.rs`
- `crates/agora/tests/cli_integration.rs`

### 11. Documentation and MCP Guidance

Append a new `MASTER_SPEC.md` section 19 subsection describing the evolved health categories, validated scan reuse, local-first Crash Doctor evidence model, memory Auto/Manual semantics, auth refresh guarantees, and launch timing/caching. Preserve historical sections 0-18.

Update both MCP skill copies so the shipped/baked guide and project agent guide agree with the ten authenticated tools:

- `.kilo/skills/agora-mcp/SKILL.md`
- `desktop/src-tauri/skills/agora-mcp/SKILL.md`

Add an optional final MCP workflow step: after local signatures, evidence, manifest context, and suspect ranking are exhausted, an AI agent may use its own web search to inspect issues in a mapped upstream repository from `read_mod_manifest.source_identifier/page_url`. It must cite results, distinguish web research from MCP output, and never create or comment on issues without explicit user approval. Do not add a launcher GitHub issue-search API.

Update Help/Guide copy for automatic evidence discovery, added files, memory Auto mode, recommendation semantics, and post-outcome guided isolation.

## Test Matrix

### Core

- Health category serialization, recommendation-only green score, stable mute keys, legacy mute import, scan token reuse, stale token rescan, changed registry invalidation, and JAR cache invalidation.
- Evidence ordering/coherence, bad timestamps, path traversal, symlink handling, file type limits, truncation, invalid UTF-8, no full-path leakage, and exact line excerpts.
- Signature typed actions, non-matches, large-log regex budget, nested exception parsing, positive-only ranking, batch query counts, and telemetry attribution semantics.
- Atomic disable-plan rollback, lazy snapshot gate, same/different fingerprint decisions, cancelled/abandoned outcomes, and session correlation.
- Memory tier boundaries, manual preservation, new-instance Auto, import behavior, resource-pack adjustment, low-RAM warning, headroom/32 GB clamps, and direct/delegated parity.
- GitHub and MSA refresh success, rotation, permanent/transient failure, concurrent callers, secure-store failure, and no-token delegated launch.
- Launch trace phase completeness, health/Java/file-cache hit counts, deterministic bounded materialization order, and snapshot reference comparison without blob rereads.

### CLI

- Existing launch success/crash/JSON tests remain keychain-free.
- Full health output includes recommendations without failing or prompting.
- Realistic crash fixtures listed in section 7 pass in human and JSON modes.
- Repeatable `--file`, nonexistent instance, unsafe file, oversized file, and no-evidence behavior are deterministic.
- `instance recommend-memory` explains factors and never changes settings.
- `launch --timings` keeps timing output separate from JSON stdout.

### Desktop E2E

- Muted warnings plus no blockers launch without Health Check.
- Recommendation-only report launches without Health Check.
- Mixed warning/recommendation dialog starts with recommendations collapsed; group mute persists.
- `ERR_MSA_AUTH_REQUIRED` closes Health Check and exposes the Microsoft sign-in action.
- Other failed health-approved launches remain visible and recoverable.
- Troubleshoot auto-collects without paste, displays selected source, adds/removes browsed files, and retains paste fallback.
- Instance Editor Investigate button opens the same flow.
- Curated fix, evidence view, suspect ranking, lazy snapshot, dependency-aware disable, launch interruption/resume, same/different crash, confirmation, restore-all, and AI-offline behavior.
- Crash alert targets the correct exited instance and never auto-opens a stale unrelated modal.
- Auto memory UI is explainable; existing Manual instances remain unchanged.

### Validation Commands

Run before source changes:

```powershell
cargo test -p agora-cli --test cli_integration
npm run test:e2e -- e2e/health-launch.spec.ts e2e/crash-investigator.spec.ts e2e/launch-events.spec.ts
```

Run focused suites during implementation:

```powershell
cargo test -p agora-core
cargo test -p agora-cli --test cli_integration
npm run build
npm run test:e2e -- e2e/health-launch.spec.ts e2e/crash-investigator.spec.ts e2e/launch-events.spec.ts e2e/instances-create-launch.spec.ts
```

Run final validation:

```powershell
cargo test --workspace
npm run build
npm run test:e2e
python compiler/test_compile.py
python compiler/test_compile_integration.py
```

Run desktop npm commands from `desktop/`. Run registry compilation/database verification through the repository `/registry` workflow after changing crash signatures. Use only temporary output paths for benchmark and compiler artifacts.

## Acceptance Criteria

- No Health Check dialog appears for recommendation-only reports or when every warning is muted and there are no blockers.
- Microsoft sign-in errors are visible immediately after health approval and cannot remain covered by Health Check.
- An unchanged desktop launch performs one full health scan, not two; stale input cannot reuse approval.
- Troubleshoot and Instance Editor Investigate require no copy/paste and automatically analyze the most useful recent local evidence.
- Crash Doctor cites evidence, integrates curated fixes, safely tests one change set, waits for the correlated outcome, and always offers recovery before destructive testing.
- CLI realistic crash fixtures exercise the same core collector and result model used by desktop.
- Existing instances retain their memory settings; new Auto instances receive a bounded, explained recommendation from pack size proxies.
- GitHub and Microsoft credentials survive ordinary access-token expiry and transient outages; only permanent refresh failure signs the user out.
- Delegated launch works without in-app MSA credentials.
- Warm launch traces show eliminated duplicate health work and Java probes, while cold materialization uses bounded parallelism without weakening verification.
- No crash evidence, account secret, or full browsed path is logged, uploaded, rendered as raw HTML, or exposed through a generic filesystem command.

## Explicitly Out of Scope

- Built-in GitHub issue search, issue submission, or curated issue governance.
- Remote crash telemetry or any hosted diagnostic service.
- Automatic modification of Manual memory settings.
- Unattended destructive Crash Doctor actions from CLI or MCP.
- Removing saves from recovery snapshots.
- Replacing Tauri, SQLite, the component library, or the canonical launch controller.
