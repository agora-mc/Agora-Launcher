# Interactive Experiences: Implementation Status

Status: **COMPLETE — SOL-4 SHIP recommendation (2026-08-11).** Every gate in the coordination plan has run and passed: SOL-0 → DEEPSEEK-1..7 → SOL-1 → SOL-2 (§15–§20) → TERRA-5 → SOL-3 → TERRA-6/6b → **SOL §22 rulings** → **fix batch** → **TERRA-7 retest (PASS)** → **LUNA-5 sweep (PASS)** → **SOL-4 (§23, SHIP)**.

All 5 TERRA-6 P1s are closed and independently retested, including the two §22 BLOCKERs found against a real 136-mod instance. Both SOL-3 P2s are closed. Gates: unit **211/211**, e2e **243/243**, boundary 63 files + 24/24 fixtures, `npm run build` green, `cargo check -p agora-desktop` clean, root hermetic checks OK.

**Owed before tagging a release:** the full viewport / density / high-contrast / 200%-text / reduced-motion matrix (LUNA-5 note 3) — a verification gap, not a known defect. **Phase 2 priorities** are ordered in `ARCHITECTURE_REVIEW_SOL.md` §23.8, led by unblocking install/update (§19.5), without which High Interaction offers only the destructive content verb.

Authoritative contracts: `MASTER_ARCHITECTURE.md`, `DOMAIN_MODELS.md`, `VISUAL_LANGUAGE.md`, `LESSON_MAP.md`, `SAFETY_BOUNDARIES.md` (all SOL-0, present), `ARCHITECTURE_REVIEW_SOL.md` (SOL-1 APPROVED; SOL-2 gate §15 blocked with partial seam approval; re-reviews §16/§17/§18/§19 all BLOCKED then remediated in batches 1–5; **§20 fifth re-review APPROVED — gate passed**).

## Where we are in the plan (read this first)

The batches executed after SOL-2 were **gate-fix iterations on the DEEPSEEK-6 read-only layer**, NOT the plan's DEEPSEEK-7 / DEEPSEEK-8 phases.

| Plan phase | Status | What exists today |
|---|---|---|
| DEEPSEEK-6 — live read-only adapters | Done | read-only High Interaction surface (instance, content/relationships, health, snapshots, crash evidence, runtime) with fragment-based availability (failed reads render unavailable/unknown, never ready) |
| SOL-2 live-operation safety gate | **PASSED (§20)** | five reviews (§15–§19) BLOCKED then remediated in batches 1–5; §20 fifth re-review APPROVED |
| **DEEPSEEK-7 (plan) — High Interaction live operations** | **Done** | The Sol-approved seams are ENABLED (`liveHighInteractionCapabilities`): remove via InstallFlow, review-only health inspection, loader plan/chooser/change, read-only snapshot compare, and Crash Doctor navigation. Each contextual bridge (`live/operationBridges/`) is wired into InstanceEditor and verified end-to-end (new `high-interaction.spec.ts` covers the read surface + health dialog + remove→InstallFlow; **Terra TERRA-5 independently PASSED** the live surface 2026-08-10 — every approved seam preserves authority, every rejected seam is absent). Install/update stays blocked on a fresh curated candidate/target-version source (§19.5). Rejected seams (dependency disable, snapshot restore, direct health repair, direct crash experiments, memory) remain OFF and bridge-less. |
| **DEEPSEEK-8 (plan) — scale and maintain** | **NOT started** | no perf / large-graph simplification / virtualization / refactor / docs-maintenance work has been done |

**Current position:** **SOL-2 gate PASSED** (§20), **DEEPSEEK-7 DONE**, **TERRA-5 PASSED**, and **SOL-3 APPROVED** (§21, 2026-08-10). Remaining per the coordination plan: **LUNA smoke/regression** on the live High Interaction surface (pending) + **TERRA final UX**, then **DEEPSEEK final fixes** (incl. the 2 SOL-3 P2 refinements + DEEPSEEK-8 scale/maintain polish), then **SOL-4**. Install/update gestures remain blocked on a fresh curated candidate/target-version source (§19.5); rejected seams stay absent.

**Naming:** the five remediation batches below are "SOL-2 gate fixes — batch 1 / batch 2 / batch 3 / batch 4 / batch 5" (batches 1–2 historically labelled DEEPSEEK-7 / DEEPSEEK-8 in earlier handoffs). Those historical labels are NOT the plan's DEEPSEEK-7 / DEEPSEEK-8 phases.

## Phase log

| Phase | Agent | Status | Notes |
|---|---|---|---|
| SOL-0 architecture | Sol | Done | five contract docs under `docs/interactive/`; untracked in git |
| DEEPSEEK-1 repo mapping | DeepSeek | Done | this document |
| DEEPSEEK-2 visual framework | DeepSeek | Done | domain/ + visual/ + import-boundary check |
| DEEPSEEK-3 vertical slice | DeepSeek | Done (HARD STOP) | Build It, Mod It, Undo It + Lab shell + tests |
| Terra UX gate (TERRA-1..3) | Terra | Done | `UX_FINDINGS_TERRA.md`; verdict: do NOT proceed until P1/P2 fixed |
| DEEPSEEK-4 Terra fixes | DeepSeek | Done | all 5 P1 + P2 fixed; Terra TERRA-4 PASS |
| SOL-1 architecture gate | Sol | Done (APPROVED) | 4 BLOCKERs + 2 residual batches fixed by DeepSeek; SOL-1 complete |
| DEEPSEEK-5 remaining adventures | DeepSeek | Done | Heal It, Fix It, Take It Offline + 5 shared visuals |
| Luna regression (Lab batch) | Luna | Done | all six adventures passed; L-001 (overflow) fixed by DeepSeek and retested by Luna |
| DEEPSEEK-6 live read-only adapters | DeepSeek | Done | `interactive/live/` boundary + adapters + host + FIX-BEFORE-LIVE closures |
| SOL-2 live-operation safety gate | Sol | Done (BLOCKED, partial approval) | `ARCHITECTURE_REVIEW_SOL.md` §15; 5 cross-cutting blockers assigned to DeepSeek |
| SOL-2 gate fixes — batch 1 (DEEPSEEK-6 remediation) | DeepSeek | Done | 5 cross-cutting blockers + §15.10 corrections on the READ-ONLY layer; NOT the plan's DEEPSEEK-7 |
| SOL-2 bounded re-review | Sol | Done (BLOCKED) | `ARCHITECTURE_REVIEW_SOL.md` §16; BLOCKERs A–D + §16.6-16.7 assigned to DeepSeek (16.9 handoff) |
| SOL-2 gate fixes — batch 2 (DEEPSEEK-6 remediation, §16) | DeepSeek | Done | BLOCKERs A–D + §16.6 corrections + all consequential capabilities off + contextual bridge scaffolding; NOT the plan's DEEPSEEK-8 |
| SOL-2 second bounded re-review | Sol | Done (BLOCKED) | `ARCHITECTURE_REVIEW_SOL.md` §17; BLOCKERs A–C + §17.6 gaps assigned to DeepSeek (17.9 handoff) |
| SOL-2 gate fixes — batch 3 (DEEPSEEK-6 remediation, §17) | DeepSeek | Done | reversible/latest-wins canonical state, discriminated action-bearing bridges + InstallFlow, aggregated alias-safe Tauri allowlist; see fix log below |
| SOL-2 third bounded re-review | Sol | Done (BLOCKED) | `ARCHITECTURE_REVIEW_SOL.md` §18; BLOCKERs A–D assigned to DeepSeek (18.9 handoff) |
| SOL-2 gate fixes — batch 4 (DEEPSEEK-6 remediation + Rust backend, §18) | DeepSeek | Done | typed availability gate (locks/recovery/process/install), leave-High-Interaction terminal lifecycle, reviewOnly hardens HealthDialog, atomic install launch-exclusion in Rust; see fix log below |
| SOL-2 fourth bounded re-review | Sol | Done (BLOCKED) | `ARCHITECTURE_REVIEW_SOL.md` §19; BLOCKERs A–B (launch→install race, instance-identity binding) assigned to DeepSeek (19.7 handoff) |
| SOL-2 gate fixes — batch 5 (DEEPSEEK-6 remediation + Rust backend, §19) | DeepSeek | Done | target-aware launch/install mutual exclusion in all three launch entries; scene bound to loaded instance ID with render-time withhold guard; per-route fresh re-resolve in the install bridge; see fix log below |
| SOL-2 re-review (5th) | Sol | **Done (APPROVED)** | `ARCHITECTURE_REVIEW_SOL.md` §20; both §19 blockers verified closed; gate PASSED — DEEPSEEK-7 authorized |
| **DEEPSEEK-7 (plan): High Interaction live operations** | DeepSeek | **Done** | approved seams enabled + bridges verified end-to-end (see DEEPSEEK-7 fix log below); TERRA-5 independently PASSED the live surface; install/update blocked on curated source |
| Terra deep live UX review (TERRA-5) | Terra | **Done (PASS)** | `UX_FINDINGS_TERRA.md` TERRA-5 (2026-08-10): no P0/P1; P2 Runtime "Unknown" legibility + P3 first-paint crowding (both non-gating) |
| Luna smoke/regression (live surface) | Luna | Not started | pending; TERRA-5 was performed ahead of it (documented in UX_FINDINGS_TERRA.md) |
| SOL-3 educational correctness | Sol | **Done (APPROVED, 2 P2)** | `ARCHITECTURE_REVIEW_SOL.md` §21 (2026-08-10): all six adventures teach correct mental models vs the Guide; 2 P2 refinements assigned to DeepSeek (Fix It wrong-hypothesis revision; Heal It Quilt indeterminate next-step) |
| **DEEPSEEK-8 (plan): scale and maintain** | DeepSeek | **Not started** | final-polish/scale phase; due as the "DEEPSEEK final fixes" after SOL-3 + the final Terra/Luna pass, before SOL-4 |
| **TERRA-6 final UX** | Terra | **Done (DO NOT PROCEED)** | `UX_FINDINGS_TERRA.md` TERRA-6 (2026-08-11): **4 P1** — T6-1 Mod It conflict invisible on the primary path (`reduce()` never sets `conflictVisible`); T6-2 resume silently reverses a player decision; T6-3 `InstanceBench:127` renders every non-compatible loader state unmarked, and `readAdapters:68` sets live loader compatibility to `unknown` unconditionally — so **every real instance's current loader renders unmarked**, indistinguishable from verified-good (**shared visual — reaches the live surface**, → Sol; sibling of the fixed TERRA-5 runtime P2); T6-4 High Interaction Mode has no settings control (violates `MASTER_ARCHITECTURE.md:139`) and self-disables via the §18.4 bridges. **TERRA-6b real-app pass** (read-only, real 136-mod instance, debug build): T6-3 and T6-4 CONFIRMED live, plus **T6-11 (P1)** — `readAdapters:97,100` compare `mod_id === registry_id` where both are `string \| null`, so one unattributed warning marked **all 136 mods "Warning"** while the same screen said `0 Need attention` / `1 warning`; the identical blocker arm would mark them **Blocked**. Also T6-12 findings render twice + all recommendations hardcoded `structuredKind:'runtime'`/`affectedIds:[]`; T6-13 nodes labelled by raw jar filename (less legible than Standard). Total 5 P1 / 8 P2 / 9 P3. SOL-3's two P2s independently confirmed still open. Passed live: 12-node cap + "Show all 136", health bridge reviewOnly with 0 Disable/Fix/Repair (§18.5 holds). |
| **SOL §22 escalation rulings** | Sol | **Done** | `ARCHITECTURE_REVIEW_SOL.md` §22: T6-11 and T6-3 ruled **BLOCKER**; T6-4 split approved (session view state ≠ persisted preference; §18.4 preserved) + missing §5.2 control ruled a contract omission; §22.6 records the fixture-coverage lesson |
| **DEEPSEEK final fixes (incl. DEEPSEEK-8 polish)** | DeepSeek | **Done** | all 5 P1 + T6-7/8/9/12/13/14 + both SOL-3 P2s; 2 approved contract additions (`VisualContentNode.fileLabel`, `LabScenario.safeResumeCheckpoint`); +20 tests (211/211) incl. the mandatory null-identity fixtures |
| **TERRA-7 retest** | Terra | **Done (PASS)** | `UX_FINDINGS_TERRA.md` TERRA-7: every P1 verified black-box on the real 136-mod instance and in the Lab; golden standard now fully met; no open findings |
| **LUNA-5 release-candidate sweep** | Luna | **Done (PASS)** | `VISUAL_REGRESSION_LUNA.md` (new): all suites green, real-instance visual sweep, no Standard-mode regression; one owed item (full a11y/viewport matrix) |
| **SOL-4 final review** | Sol | **Done (SHIP)** | `ARCHITECTURE_REVIEW_SOL.md` §23: ship the Lab and High Interaction as an opt-in presentation; null/unknown fixtures now a standing requirement; Phase 2 priorities ordered |
| SOL-4 final architecture/release review | Sol | Not started | final |

## DEEPSEEK-5 remaining-adventures batch log

New shared visuals (`desktop/src/features/interactive/visual/`): `HealthLens` (blocker/warning/recommendation hierarchy + validated summary), `LoaderRail` (roles, proven-vs-indeterminate, requirement counts), `RuntimeWorkbench` (runtime/Java, memory current/recommended/proposed + headroom, GC), `CrashEvidenceBoard` (clues, hypotheses with strength, experiment region, privacy note), `NetworkReadinessMap` (per-category ready/missing/blocked-by-policy/unknown, overall). All are controlled components emitting `VisualIntent`s only (HealthLens emits `review-loader` via finding reviewIntent; LoaderRail `review-loader`; RuntimeWorkbench `propose-memory`; NetworkReadinessMap `review-offline-readiness`).

New adventures (`desktop/src/features/interactive/lab/scenarios/`), all deterministic, namespaced `lab:*`, zero real mutation:

- **Heal It** (`heal`) — run validation sweep → fix loader blocker (proven-compatible vs indeterminate vs blocked) → decide Java warning (managed vs keep, warning persists) + memory (automatic vs manual with headroom) → re-run green. No `javafml`/resolver internals exposed. Guides: `launching`, `java-performance`.
- **Fix It** (`fix`) — read evidence → pick hypothesis (high/medium/low, never certainty) → serious-confirm a one-change experiment with a recovery point → interpret outcome (supporting / less-likely / changed) → confirm recovery (one launch ≠ proof) or restore from the recovery point on cancel. Guides: `crash-recovery`.
- **Take It Offline** (`offline`) — inspect readiness (game files, loader, content, Java, sign-in/launch) for delegated launch → prepare missing content → choose launch mode (delegated ready vs direct blocked-by-policy for sign-in) → re-check, distinguishing Ready from Unknown. Simulation-only (no truthful live aggregate query exists yet). Guides: `privacy-offline`.

Wiring: `simulationAdapter` registers all 6 adventures; `ScenarioView` adds `heal`/`fix`/`offline` cases; `labCapabilities` now enables `canReviewHealth`, `canReviewLoader`, `canProposeMemory`, `canReviewOfflineReadiness`.

Tests added: `scenariosRemaining.test.ts` (happy path + mistake recovery + intent mapping for all three), `newVisuals.test.tsx` (render + intent-emission smoke tests for all five visuals), LabShell end-to-end playthroughs for all three adventures, and the scenario-contract suite auto-covers the new adventures (namespaced IDs, determinism, simulation source, valid guide/destinations, reachable decisions).

Open non-gate items (carried forward): Terra P3 wordmark polish; SOL-1 FIX BEFORE LIVE MODE + SAFE DEBT lists (route-level lazy loading is now more relevant with 6 adventures — the SAFE DEBT 3 note still applies).

Tests added: `decisionGate.test.ts` (gate dispatch/confirm/reject + confirm-time revalidation incl. stale/disabled), LabShell route-parity + modal exclusivity + focus-return tests, `simulationAdapter.test.ts` fixtures-mode boundary test, `visualContract.test.ts` (no operation-like callbacks, review intent in union). ChangeStaging component tests assert the exact `review-staged-changes` payload.

## DEEPSEEK-5b fix log (SOL-1 re-review residual batches -> fixes)

| Residual | Fix |
|---|---|
| A — boundary check still fail-open | `checkFile` now REJECTS unclassified sources under the scan root (only `live/` is exempt) instead of silently returning. Classification and target containment are now path-SEGMENT based (`relative(root, file)` — no `../`/absolute) instead of raw string prefixes, so a sibling like `interactive-app/` can never classify as `lab/` or be accepted as internal. The shared-visual callback guard is now a TypeScript AST walk that rejects BOTH property signatures (`onReview?: …`) and METHOD signatures (`onReview(): void`). Added 3 negative fixtures: `unclassified-source.ts` (root-level, app import), `lab/fixture-prefix-collision.ts` (imports `../../interactive-app/lab/controller`), and `visual/fixture-callback-method.ts` (`onReview(): void` interface), plus the `interactive-app/lab/controller.ts` resolution target. Fixtures mode now asserts 11/11 files flag. |
| B — confirmation not bound to reviewed consequence | `revalidateDecisionForConfirm` now takes a `DecisionConsequence { danger, confirmTitle, confirmBody }` snapshot (what the player reviewed at open time) and compares it against the re-resolved CURRENT decision immediately before dispatch. If the decision disappears, becomes disabled, OR its danger/title/body changed, the old confirmation closes with safe feedback and does not dispatch. `LabShell.confirmPendingDecision` passes `pendingConfirm`'s consequence. Added same-ID changed-consequence tests (title change, body change, danger flip) proving no dispatch authorization, and a `userEvent` Tab/Shift+Tab focus-containment test for the dialog. |

Open non-gate items (carried forward): Terra P3 wordmark truncation (app-shell polish); SOL-1 FIX BEFORE LIVE MODE list (freshness fail-closed, uncertainty rendering, ContentGraph perf, snapshot ordering/availability) and SAFE DEBT list (typed scenario registry, feedback single-source-of-truth, route-level lazy loading, cross-contract guide/destination check) — none block the SOL-1 re-review.

## DEEPSEEK-6 batch log (live read-only adapters + FIX-BEFORE-LIVE closures)

New `desktop/src/features/interactive/live/` boundary (the only app-boundary layer):

- `readAdapters/index.ts` — pure DTO→presentation adapters for instance, content/relationships, health, snapshots, crash evidence, runtime. All REDACT private fields (sha256, paths, scan tokens, fingerprints, raw logs, breakdowns, JVM args) and preserve `unknown`/`indeterminate`.
- `freshness.ts` — `liveSource`/`nextRevision`/`isExecutable`/`requiresRefresh` (only `fresh` is executable).
- `presentationPreference.ts` — versioned `standard | high-interaction` preference, safe default `standard`.
- `liveScene.ts` — `readLiveData` (per-read failure isolation) + `assembleLiveScene` (fresh scene with revision).
- `LiveSceneView.tsx` + `LiveInteractiveHost.tsx` — the High Interaction read surface: loading / scene / empty / error states (fail-closed), a Refresh with new revision, an always-present **Use Standard view** escape, and intent routing that forwards review intents to the STANDARD surface (never executes here). Injectable `load` for tests.
- `liveCapabilities.ts` — read-only capabilities (no propose/restore/memory actions; review/health/loader/crash/snapshot-compare enabled to route to Standard).

FIX BEFORE LIVE MODE closures:
1. **Freshness fail-closed** — `domain/guards.ts`: `refreshing` is no longer executable (`freshnessGate` rejects it); `live/freshness.ts` isExecutable only for `fresh`. Duplicate/unavailable actions: `ContentGraph` no longer stages an action when an identical proposal exists or the node is `locked`/`busy`/`unavailable` (persistent reason shown); `RecoveryTimeline` compare/restore gated by snapshot availability with a reason. Single source authority: `ContentGraph` + `ChangeStaging` dropped their separate `source` prop (scene carries its own source).
2. **4-state relationships** — `ContentGraph` renders satisfied/missing/conflicting/indeterminate/unknown explicitly in both diagram (socket chips) and linear views; state is never inferred from `toId` or prose.
3. **ContentGraph large-instance readiness** — memoized node map + relationship grouping per scene revision, spatial viewport capped at 12 with "Show all N", linear view complete + searchable, roving focus (arrow-key navigation) with stable focus restore on refresh.
4. **Snapshot ordering/availability** — `VisualSnapshot` gained authoritative `sortKey` (ISO) separate from the display `createdAt`; `RecoveryTimeline` sorts by `sortKey ?? createdAt` with stable id tie-break and gates compare/restore by availability.

Wiring: `InstanceEditor` gained a contextual **High Interaction view** toggle (renders `LiveInteractiveHost` for the same instance, `Use Standard view` escape, crash intents open the standard CrashInvestigator). No mutation is wired anywhere.

Tests: `readAdapters.test.ts` (DTO redaction + mapping + uncertainty), `freshness.test.ts`, `presentationPreference.test.ts`, `LiveInteractiveHost.test.tsx` (loading/scene/empty/error, refresh revisions, review-intent routing, Standard escape), `boundary.test.ts` (lab/visual/domain never import live; live imports only tauri/domain/visual/react), plus updated guards/ContentGraph/RecoveryTimeline tests. Unit 137/137, boundary OK (57 files) + 11/11 fixtures, build green, e2e 241/241.

### Intended future operation seams (recorded, NOT wired — SOL-2 §14.3.7)

| Proposed live interaction | Existing authoritative Agora operation + review/recovery seam |
|---|---|
| Install / update content (ContentGraph gesture → staging) | `InstallIntent` → `InstallFlow` → canonical install pipeline (dependency/conflict/file/snapshot review, fingerprint, rollback, health) |
| Dependency-aware disable/remove | Existing dependency plan + `DependencyPrompt` (dependent-aware review) |
| Loader change (LoaderRail candidate → review) | `planLoaderChange` → `LoaderChooser` / loader change command + post-change health refresh |
| Health/launch review (HealthLens blocker) | `checkInstanceHealth` scan token → `HealthDialog` (blocker override semantics) |
| Snapshot compare/restore (RecoveryTimeline) | `detectDrift` diff + existing restore command behind serious confirmation (pre-restore undo point, process exclusion) |
| Crash experiment (CrashEvidenceBoard) | `CrashInvestigator` recovery-first experiment flow (recovery snapshot before first mutation, restore on abandon) |
| Memory/runtime change (RuntimeWorkbench) | `recommendInstanceMemory` + `updateInstanceJvm` after summary review |

No live gesture calls any of these today; the live surface only opens the Standard view (or the standard CrashInvestigator) where the authoritative flow lives.

## SOL-2 gate fixes — batch 1 log (DEEPSEEK-6 remediation; historically labelled "DEEPSEEK-7" — NOT the plan's DEEPSEEK-7)

| BLOCKER | Fix |
|---|---|
| 1 — partial read failures become false fresh/healthy | `liveScene.ts` now returns `Fragment<T>` (ok/error) per read; errors are NEVER erased into empty values. Aggregate freshness is derived from the fragments (`derivedFreshness`: any error → `unknown`, never `fresh`). A failed instance read yields an empty scene and the host reports an error (never a valid empty instance). `instanceToVisual` treats process-state uncertainty as `busy` (never editable). `contentToVisual` marks nodes `unknown` (never healthy) when the health read failed. `HealthLens` gained an `unavailable` state ("could not be verified — nothing is treated as ready"); snapshots/runtime fragments render unavailable notes instead of empty/ready. Host/LiveSceneView pass per-fragment availability. Tests: per-fragment failure + total failure via `readLiveData`-level fragments and the default loader path (`buildHostData`). |
| 2 — unused intent gates / no controller | New `live/intentController.ts` owns the sequence: source → capability → freshness (non-fresh → `refresh-required`) → in-flight/availability → approved bridge. `routeLiveIntent` is the single routing path the host calls for EVERY intent; rejected seams (disable, restore, memory) are capability-blocked and never routed. `operationSeamFor` records the authoritative operation per bridge. Tests: selection/navigation, approved review routing, rejected-seam blocking, refresh-required, missing-source, in-flight coalescing, seam mapping. |
| 3 — capability flags advisory | Shared visuals now gate every operation-shaped command: `HealthLens` Review buttons render only when the finding's review intent capability is enabled; `RuntimeWorkbench` memory buttons require `canProposeMemory` (with an unavailable reason otherwise); `RecoveryTimeline` Compare requires `canPreviewSnapshot`; `CrashEvidenceBoard` renders its Open Crash Doctor button only under `canOpenCrashDoctor`. The controller enforces the same independently. Added both-state component tests. |
| 4 — refresh races + canonical state | `LiveInteractiveHost` is latest-wins (request token discards out-of-order results), keeps the last scene visible marked `refreshing` while reloading (never hides to a bare loading screen), resets revision/selection on instance change, and consumes the canonical `processState` + `installActive` props (from `useProcessController` / pack-install task via InstanceEditor). Tests: latest-wins out-of-order, refresh-keeps-scene-visible, canonical running → Running/Busy. |
| 5 — over-broad live/ exemption | `check-interactive-boundaries.mjs` now classifies live subareas: `read` (readAdapters + liveScene) may invoke only the `@/lib/tauri` READ-command allowlist; `core` (host, view, capabilities, preference, freshness, controller) may use tauri types only; `operationBridges/` may host Standard controllers but not invoke tauri directly; unknown live files fail. Negative fixtures: `live/readAdapters/fixture-mutation.ts` (restore in read layer) and `live/fixture-unknown.ts` (unclassified live file). Fixtures now 13/13. |
| §15.10 preference | `presentationPreference` is now WIRED: `InstanceEditor` initializes High Interaction from `loadPreference()` and persists via `savePreference()` (versioned, default standard). |
| §15.10 honest copy / fallback | LiveSceneView copy now reads "Read-only — enabled review actions open the Standard view"; unsupported/unavailable fragments fall back to Standard-reachable notes; the host always offers the Standard escape. |

Approved bridges (InstallFlow planning, health review, loader review/change, snapshot compare, Crash Doctor navigation) are implemented as DISABLED scaffolds: gestures route through `routeLiveIntent` to the Standard surface (crash → standard CrashInvestigator; all other review intents → Standard view), with all consequential capabilities off until Sol verifies them. Rejected seams (disable, health repair, restore, crash experiments, memory) remain absent.

> **Correction (DEEPSEEK-8, per Sol §16.3):** the above "disabled scaffolds, all consequential capabilities off" claim was WRONG at DEEPSEEK-7 time. `liveCapabilities.ts` then defaulted `canReviewHealth`, `canReviewLoader`, `canOpenCrashDoctor`, and `canPreviewSnapshot` to `true`, so those review controls WERE reachable in the shipped High Interaction surface, and `InstanceEditor`'s callback only switched to Standard view (losing context) instead of opening the named review. DEEPSEEK-8 defaults EVERY consequential live capability to `false` and implements the actual contextual bridges (see the DEEPSEEK-8 fix log below). The current statement that matters: no consequential live capability is enabled; no rejected-seam bridge exists.

Tests: `intentController.test.ts`, `liveScene.test.ts` (fragments), updated `LiveInteractiveHost.test.tsx` (latest-wins, canonical state, health-unavailable), updated `HealthLens`/`RecoveryTimeline`/`newVisuals` capability tests, boundary fixtures. Unit 152/152 (21 files), boundary OK (60 files) + 13/13 fixtures, build green, e2e 241/241.

## SOL-2 gate fixes — batch 2 log (DEEPSEEK-6 remediation, §16; historically labelled "DEEPSEEK-8" — NOT the plan's DEEPSEEK-8)

Maps every finding in `ARCHITECTURE_REVIEW_SOL.md` §16.2–16.7 to its fix. All consequential live capabilities stay OFF (`liveCapabilities.ts` returns every flag `false`); no rejected-seam bridge was added.

| Finding | Fix |
|---|---|
| §16.2 A — refresh still executable; latest-wins not instance-safe; request-id collision; only `running` projected; no canonical conflict gate; missing deterministic tests | `LiveInteractiveHost.tsx` rewritten: one monotonic `requestRef` generation, NEVER reset (instance switches cannot collide); `instanceIdRef` captured and verified before applying resolved data; `refresh()` marks the RETAINED scene source `liveSource(revision, 'refreshing')` (non-executable to `routeLiveIntent`) and the accepted read stores `refreshing: false` (never left true); instance-change reset is isolated in an effect depending only on `[instanceId]`; canonical state is re-applied by a separate effect on `[processState, installActive, applyCanonical]` WITHOUT reloading/resetting; `projectLaunchState` maps launching/starting→starting, running→running, stopping→stopping, delegated→delegated, failed→failed; `handleIntent` passes `{ busy: lockState==='busy' || installActive }` into `routeLiveIntent`. New deterministic tests: overlapping refresh (oldest unresolved discarded), instance switch with old promise unresolved, in-flight review not executed during refresh, all canonical phases (launching/running/stopping/delegated/failed), install-conflict blocked, different-instance process ignored. |
| §16.3 B — contextual bridges absent while four capabilities enabled; host discarded bridge; InstanceEditor only exited High Interaction | `live/operationBridges/index.ts` (NEW): `BridgeContext` union (health-review / loader-review / snapshot-compare / crash-doctor / install-flow), `LiveReviewRoute`, `StandardBridgeHandlers`, `openBridge(route, handlers)` dispatching to the named Standard surface. `intentController.routeLiveIntent` now returns `{ status:'review', bridge, intent, context }` with minimal typed context (`contextForBridge`), and takes `(scene, intent, capabilities, instanceId, conflict)`; `capabilityGate` gates `review-staged-changes` by `canProposeInstall`. Host forwards `{ bridge, context }` via `onOpenStandardOperation(route)` — never discards it. `InstanceEditor` builds `StandardBridgeHandlers` via `openBridge`: health-review → fresh `checkInstanceHealth` then `onReviewHealth`; loader-review → `openLoaderChooser()`; snapshot-compare → `detectDrift` + `setSnapshotDiff` + `setActiveTab('snapshots')`; crash-doctor → `onInvestigate`; install-flow → `onOpenBrowseForInstance`. Each bridge performs its own fresh read/re-resolve. `liveCapabilities` defaults EVERY consequential capability to `false`. Tests: controller review routing with context; all default capabilities keep every consequential intent blocked. |
| §16.4 C — live boundary fail-open on relative `./`; dynamic/re-export/import-equals treated as type-only; only 2 live fixtures | `check-interactive-boundaries.mjs`: every live specifier (relative `./`/`../`, `@/`, static/dynamic/re-export/import-equals) is now RESOLVED with the TS resolver; resolved files enforce interactive containment + allowed edge direction per subarea (`read`→read/domain/visual; `core`→read/core/bridges/domain/visual; `bridges`→read/core/bridges/domain/visual; react only externally, `@/lib/tauri` handled separately); `tauriImportForm` classifies named/namespace/side-effect/dynamic/reexport-star/import-equals — anything unverifiable is a violation, never treated as type-only; `freshness.ts` reclassified as read-pipeline. New fixtures: install, disable, launch, settings mutations in the read layer, a relative-path app-layer bypass, a dynamic-import bypass, a re-export bypass, and a bridges-layer tauri fixture. Fixtures now 21/21. |
| §16.5 D — runtime rendered even when memory/Java reads failed; crash evidence falsely said recovery ready; no default-command failure tests | `buildHostData` returns a runtime `err` fragment when detail OR memory OR javas failed; `LiveSceneView`/`RuntimeWorkbench` render the unavailable note whenever `runtime.availability === 'unavailable'` (no unsupported fallback facts); `crashToVisual` sets `recoveryReady: false` (recovery is created by Crash Doctor before any experiment — never claimed at read time) and `readAdapters.test.ts` updated; NEW `liveDefaultLoader.test.ts` mocks `@/lib/tauri` and forces each default command (`getInstanceDetail`, `queryLaunchState`, `checkInstanceHealth`, `listSnapshots`, `investigateInstanceEvidence`, `recommendInstanceMemory`, `listJavaRuntimes`) to fail individually and all together through `readLiveData → defaultLiveLoad → buildHostData`, asserting fragment states. |
| §16.6 — adapter superlinear; copy said simulated / in use | `contentToVisual` normalized to O(nodes + relationships) via `byFilename`/`idToNodeId` maps with single-pass summary accumulation; `HealthLens` empty copy now "No findings — this instance is ready to launch." (simulated wording removed); `RuntimeWorkbench` says "GB configured" (was "in use"); the view still does not imply a complete dependency graph. |
| §16.7 — every consequential integration NO; rejected seams unchanged | Confirmed: `liveCapabilities()` returns every capability `false`; no contextual bridge exists for a rejected seam (disable, restore, memory, direct repair, direct crash experiments). |

Tests: unit 167/167 (22 files, +15: controller context/off-by-default, host BLOCKER A behaviors, default-loader failures), production boundary OK (62 files), fixtures 21/21, `npm run build` green, e2e 241/241.

## SOL-2 gate fixes — batch 3 log (DEEPSEEK-6 remediation, §17; closes the SOL-2 second bounded re-review)

Maps every finding in `ARCHITECTURE_REVIEW_SOL.md` §17.2–17.6 to its fix. All consequential live capabilities stay OFF (`liveCapabilities.ts` returns every flag `false`); no rejected-seam bridge was added.

| Finding | Fix |
|---|---|
| §17.3 A — canonical busy sticky (idle→launching→idle never clears) and regressed by stale acceptance-time closures; completed/failed install progress treated as active; no deterministic overlap/transition tests | `LiveInteractiveHost.tsx` now stores the BASE (unprojected) read scene in state and projects canonical state at RENDER via `projectCanonical(scene, {processState, installActive, instanceId})` (pure + reversible: busy is derived ONLY from canonical state; the base read/player lock state is restored when it clears). No acceptance-time closure captures canonical values — a canonical change while a read is unresolved is applied the moment the accepted result lands (the projection always reads the newest props). `InstanceEditor` passes `installActive={packInstall?.status === 'running'}` (only a RUNNING install task is active; completed/failed are not). New deterministic tests: idle→launching→idle clears busy; install active→inactive clears busy; canonical change during an unresolved initial load applies the LATEST state; two genuinely overlapping refreshes resolved newest then oldest (oldest discarded). |
| §17.4 B — install/update/remove context discarded (Browse-only), route not discriminated, lifecycle (in-flight coalescing + terminal refresh) absent and untested | `BridgeContext['install-flow']` now carries `action: 'install' \| 'update' \| 'remove' \| 'review'` + `contentId`; `LiveReviewRoute` is a DISCRIMINATED union (bridge and context.kind are locked together) built by `reviewRouteFor()` from the same intent, so `routeLiveIntent` returns a valid `LiveReviewRoute` (never a mismatched pair). `openBridge` passes the FULL typed context (action/contentId/filename) to `openInstallFlow(ctx)`. The host resolves the content node from the accepted live scene and enriches the context with the backend-derived `filename` + `contentKind` (node.name is the DTO filename — never parsed from a visual id). `InstanceEditor.openInstallFlow` builds an ACTION-BEARING `InstallIntent` and opens the canonical `InstallFlow`: remove → `{ type: 'remove', filename }` (fresh-verified against the manifest); install → `{ type: 'install', sourceType: 'curated', itemId: registry_id }`; update → opens the mod's Standard install review so the target version is re-selected (action+content retained, never silent Browse); unresolvable content → explicit error naming the action, never a silent Browse. Lifecycle: dispatching a review records an `in-review` proposal in the retained scene (controller's duplicate gate coalesces a second review); the next accepted read (terminal refresh) replaces the scene and clears it. Backend outcomes remain the Standard surface's authority. New tests: `operationBridges.test.ts` (every dormant adapter dispatches to the correct handler with full context), controller action-bearing routing, host in-flight coalescing + refresh-clears + install enrichment. |
| §17.5 C — Tauri allowlist last-write-wins + local-binding alias laundering + no alias/mixed fixtures | `tauriImportForm` → `tauriImportForms(source, spec)` AGGREGATES every matching import/re-export/import-equals/dynamic form (one entry per statement); `checkLiveFile` rejects when ANY form is prohibited or unverifiable (a later safe/type-only import can no longer hide an earlier mutation). Named imports now evaluate the ORIGINAL name (`element.propertyName?.text ?? element.name.text`), so `import { restoreSnapshot as getInstanceDetail }` is judged as `restoreSnapshot`. New fixtures: `fixture-alias-launder.ts` and `fixture-mixed.ts` (prohibited import followed by safe import) — both flagged as non-read `restoreSnapshot`. Fixtures now 24/24. |
| §17.6 — individual `queryLaunchState` default-failure coverage gap | Added to `liveDefaultLoader.test.ts`: `queryLaunchState` failure keeps the instance visible but process-uncertain — lockState becomes `busy` (never editable) and aggregate freshness `unknown`. |

Tests: unit 182/182 (23 files, +15: 6 host transition/overlap/lifecycle, 7 bridge dispatch, 1 controller action-bearing, 1 queryLaunchState default failure), production boundary OK (62 files), fixtures 24/24, `npm run build` green, e2e 241/241.

## SOL-2 gate fixes — batch 4 log (DEEPSEEK-6 remediation + Rust backend, §18; closes the SOL-2 third bounded re-review)

Maps every finding in `ARCHITECTURE_REVIEW_SOL.md` §18.3–18.7 to its fix. All consequential live capabilities stay OFF (`liveCapabilities.ts` returns every flag `false`); no rejected-seam bridge was added.

| Finding | Fix |
|---|---|
| §18.3 A — availability gate ignores player locks and recovery readiness (single `busy` boolean) | `routeLiveIntent` now takes a TYPED `AvailabilityInput { locked, recoveryBusy, processBusy, installBusy }` (`intentController.ts`) with a distinct explanation per condition (player lock, recovery pending/failed, active process/launch, active install). `LiveInteractiveHost` derives it from the projected scene + canonical state (locked → `locked-by-player`; recovery → `preparing`/`failed`; process → active `launchState` or base `busy` not from install; install → `installActive`). Selection/inspection stay available before the gate. Tests: controller per-state reasons + selection still passes; host blocks review for locked-by-player and preparing/failed recovery with a test-only capability. |
| §18.4 B — no owned terminal outcome-to-refresh lifecycle; manual refresh cleared the in-flight marker | Option (a): EVERY bridge now LEAVES High Interaction before Standard work begins (`InstanceEditor`: health + crash added `setHighInteractionPref(false)` first; loader/snapshot/install already did). The host unmounts, so re-entry always remounts and performs a FRESH read — no stale live state can survive a Standard close/cancel/rejection/failure/success/rollback. The host's in-review marker is no longer cleared by a manual refresh: it is re-asserted on every accepted read via `reviewInFlightRef` and is cleared only by leaving High Interaction (host unmount) or an instance change. Tests: manual refresh does NOT clear the marker (second review stays coalesced); re-mount performs a fresh read and is dispatchable again with no stale state. |
| §18.5 C — reviewOnly HealthDialog still exposed the rejected direct-disable | `HealthDialog.tsx`: `handleFixDisable` returns early when `reviewOnly` (never calls `disableModForTest`), and the Disable buttons on blockers + warnings render only when `!reviewOnly`. The High Interaction health bridge is now ORDINARY Standard navigation (leaves High Interaction first, then fresh `checkInstanceHealth` → App-level reviewOnly `HealthDialog`), so it can never reach the disable seam. The Standard e2e was updated: `health-launch.spec.ts` no longer expects a Disable button in the review-only dialog (it now asserts the review dialog retains inspection with NO Disable and zero `disable_mod_for_test` calls); the Standard disable repair remains exercised in the launch-path dialog. Tests: `HealthDialog.reviewOnly.test.tsx` — reviewOnly renders no Disable button and never calls `disableModForTest`; non-review mode keeps the Standard disable repair. |
| §18.6 D — `apply_install_plan` lacks atomic running-process/launch-reservation exclusion | New `ensure_install_apply_allowed(&AppState, instance_id)` in `desktop/src-tauri/src/commands.rs`: rejects when the TARGET instance is running (`ERR_INSTALL_PROCESS_ACTIVE`) or has a launch reservation (`ERR_INSTALL_LAUNCH_RESERVED`), and enforces one-active-install (`ERR_INSTALL_ACTIVE`). `apply_install_plan` calls it inside the SAME state-lock block that registers the install marker, so a launch cannot race between check and registration; the marker is only inserted after the checks pass. Rust integration tests: running target rejected (different instance allowed), launch reservation rejected, competing install rejected + marker untouched post-rejection, process-clear → apply allowed + registered, post-completion marker removal allows again. |

Tests: frontend unit 187/187 (24 files; +5: controller per-state availability, host locked/recovery + refresh-preserves-marker + remount-fresh-read, HealthDialog reviewOnly x2), production boundary OK (63 files), fixtures 24/24, `npm run build` green, Rust `cargo test` 68/68 (incl. 4 new install-exclusion tests), e2e 241/241.

## SOL-2 gate fixes — batch 5 log (DEEPSEEK-6 remediation + Rust backend, §19; closes the SOL-2 fourth bounded re-review)

Maps every finding in `ARCHITECTURE_REVIEW_SOL.md` §19.3–19.4 to its fix. All consequential live capabilities stay OFF (`liveCapabilities.ts` returns every flag `false`); no rejected-seam bridge was added.

| Finding | Fix |
|---|---|
| §19.3 A — process/install exclusion asymmetric (install rejects launch, but launch does not reject install; direct/recovery check-then-reserve race; delegated has no start marker) | New target-aware `active_launches: HashSet<String>` in `agora_core::state::AppState` (covers the whole delegated session — delegated has no `launch_reservation`). New shared `ensure_launch_admitted(&AppState, instance_id)` helper, used by ALL THREE launch entries at their final state-lock transition: delegated `launch_instance` registers an atomic start marker (admission + `active_launches.insert`) before entering core launch, cleared on failure and when `wait_delegated` completes; direct `launch_instance_direct` and recovery `launch_instance_with_recovery` re-check running/reservation AND `ensure_launch_admitted` under the SAME lock that sets `launch_reservation` (closes the preflight→reservation race). `ensure_install_apply_allowed` now also rejects `ERR_INSTALL_LAUNCH_ACTIVE` when the target is in `active_launches`. Existing global launch policy preserved. New Rust tests (4): launch admission rejects active install; install registered between preflight and reservation rejected at the final transition; install rejects an active delegated-launch marker then recovers after cleanup; mutual exclusion in both directions with cleanup → retry. |
| §19.4 B — instance identity not bound to rendered scene or Standard bridge; retained InstanceEditor manifest could route A content to B; missing-remove branch stranded an in-review proposal | `LiveHostState.scene` now carries the `instanceId` the scene was loaded FOR; `displayData` is withheld (renders loading, non-routable) whenever `state.instanceId !== instanceId` — a RENDER guard, not a passive reset. `handleIntent` therefore routes with an undefined scene during a transition (blocked; no A-data → B bridge). The install bridge re-resolves per route: `openInstallFlow` runs a fresh `getInstanceDetail(id)` and resolves the filename against THAT manifest (never the retained page manifest), and `beginCanonicalOperationFor(targetId, targetName, action)` targets the explicit route instance. Every rejection branch (missing item, unresolvable content, read failure) calls `setHighInteractionPref(false)` first so no in-review proposal is stranded. New deterministic host tests (2): the old scene is withheld/non-routable while a switch to an unresolved B is in flight; a shared-filename staged-install gesture after the switch resolves routes to B's instance id (never A's), and no bridge is emitted from A data during the window. |

Tests: frontend unit 189/189 (24 files; +2 host transition tests), production boundary OK (63 files), fixtures 24/24, `npm run build` green, Rust `cargo test` 72/72 (incl. 4 new launch-exclusion tests), e2e 241/241.

## DEEPSEEK-7 fix log (real High Interaction operations — SOL-2 §20 APPROVED)

The SOL-2 gate passed (§20), authorizing the plan's DEEPSEEK-7: enabling the Sol-approved High Interaction capabilities and verifying each contextual bridge end-to-end on real instances.

| Item | What was done |
|---|---|
| Enable approved seams | `liveCapabilities.ts` → `liveHighInteractionCapabilities()`: `canProposeRemove`, `canReviewHealth`, `canReviewLoader`, `canOpenCrashDoctor`, `canPreviewSnapshot` are ON. `canProposeInstall`/`canProposeUpdate` stay OFF (blocked on the fresh curated candidate/target-version source, §19.5); `canProposeEnabled`, `canRequestSnapshotRestore`, `canProposeMemory`, `canReviewOfflineReadiness` stay OFF (rejected/not approved). Function renamed for honesty (remove is no longer read-only). |
| Reachable health inspection | `HealthLens` gained a "Review health" button (header, when validated + findings exist) gated by `canReviewHealth` — previously no live visual emitted `review-health`, so the approved health bridge was unreachable. It opens the Standard reviewOnly `HealthDialog` (leave-High-Interaction lifecycle). |
| Honest copy | `LiveSceneView` copy: "Reviews open the Standard surface; content changes use the reviewed InstallFlow." (was "Read-only…"); host/read-adapter doc comments updated; `intentController` unsupported reason no longer says "read-only". |
| Verify end-to-end | New `e2e/high-interaction.spec.ts` (2 tests) with a full read-command mock: (1) High Interaction renders a real instance and "Review health" opens the Standard `HealthDialog`; (2) "Stage removal" re-resolves per route and opens the canonical `InstallFlow` with the resolved remove plan (`resolve_install_plan`). Loader / snapshot-compare / Crash Doctor bridges remain covered by `operationBridges.test.ts` + host tests. **Terra TERRA-5 (2026-08-10) independently PASSED** the live surface black-box: every approved seam preserves authority (reviewOnly dialog has zero Disable/Repair buttons; remove→InstallFlow issues only `resolve_install_plan` until `Remove Safely`; memory/restore/install-update gestures absent). |
| Test infra | `playwright.config.ts` reporter → `[['line'], ['html', { open: 'never' }]]` so a run never auto-opens a browser and the CLI exits after tests (a dead dev server + Windows webServer teardown had left the process alive). |
| TERRA-5 P2/P3 polish (2026-08-10) | P2 — `RuntimeWorkbench` no longer renders a bare, ambiguous "Unknown" `CompatibilityChip` beside the confident memory recommendation; an indeterminate Java state (genuine, not a failed read — the runtime fragment only renders when all reads succeed) is now an explicit note tied to the Runtime row: "Java runtime: not verified". P3 — `InstanceBench` stat cells gained `min-w-0` (+ `break-words` on "Need attention") so nothing sits clipped at the right edge on first paint at narrow widths. Added a `RuntimeWorkbench` test asserting the explicit label + no "Unknown" chip + the recommendation still reads. |

Tests: frontend unit 191/191 (24 files; +1 controller shipped-defaults split, +1 TERRA-5 P2 runtime-label test), production boundary OK (63 files), fixtures 24/24, `npm run build` green, e2e 243/243 (241 + 2 High Interaction).

## Screenshot assets (SOL-1 SAFE DEBT 4 note)

`web/public/screenshots/{create-instance,install-plan-review,loader-compatibility-repair,crash-doctor,privacy-lockdown}.png` show as modified in the worktree. These are DETERMINISTIC DOCUMENTATION screenshots generated by `desktop/e2e/docs-screenshots.spec.ts` (writes into `web/public/screenshots/`); the e2e run refreshed them (byte drift from re-render). They are intentional, tracked doc assets — kept, not discarded.

## Repo-wide mapping: visual model -> source DTOs -> current UI -> proposed live adapter -> Lab consumer

Legend: "live adapter" entries are proposals for DEEPSEEK-6 (read-only) / DEEPSEEK-7 (SOL-2 approved); nothing live is wired in the vertical slice.

### VisualInstance

- current source DTOs: `InstanceRow`, `InstanceManifest`, `InstanceDetail` (`desktop/src/lib/tauri.ts`), canonical process state from `useProcessController` (`desktop/src/lib/useProcessController.ts`)
- current UI: `pages/Instances.tsx`, `pages/InstanceEditor.tsx`, `components/installed-content/*`
- proposed live adapter: merge stable identity + player-facing status only; process controller wins for launch state; never expose paths/manifests
- proposed Lab consumer: `lab/scenarios/build-it` builds a fake instance on the `InstanceBench` visual

### VisualContentNode / VisualRelationship

- current source DTOs: `InstalledMod`, `InstalledContentRow`, `InstalledContentMetadata`, `DependencyDecl` (+ requirement verdicts), `PackModRow`
- current UI: `components/installed-content/*`, `components/DependencyPrompt.tsx`, `pages/InstanceEditor.tsx`, `pages/ModDetail.tsx`
- proposed live adapter: normalize nodes by ID; preserve missing/indeterminate edges; drop hashes/URLs/receipts
- proposed Lab consumer: `lab/scenarios/mod-it` uses `ContentGraph`

### VisualHealthFinding

- current source DTOs: `HealthReport` (`HealthBlocker`/`HealthWarning`/`HealthRecommendation`), `InstanceHealthScanResult`, `LoaderCompatibilityIssue`
- current UI: `components/HealthDialog.tsx`, health reports in `Instances.tsx`/`InstanceEditor.tsx`, `lib/useInstanceHealthMonitor.ts`
- proposed live adapter: map severity + structured evidence; never parse message text for behavior
- proposed Lab consumer: `lab/scenarios/heal-it` (later batch; NOT in vertical slice)

### VisualLoaderCandidate

- current source DTOs: `LoaderCompatibilityReport`, `CompatibleLoaderCandidate`, `LoaderCapabilities`, `LoaderChangePlan`/`LoaderChangeResult`
- current UI: `components/LoaderChooser.tsx`, `HealthDialog` loader switch
- proposed live adapter: preserve proven vs indeterminate and backend ranking; never expose catalog internals
- proposed Lab consumer: `lab/scenarios/heal-it` Loader Rail (later batch)

### VisualInstallPlan

- current source DTOs: resolved install plan inside `components/InstallFlow.tsx` + `lib/installFlow.ts`; plan fingerprint stays private
- current UI: `components/InstallFlow.tsx`, `components/DependencyPrompt.tsx`, `components/PackInstallProgress.tsx`
- proposed live adapter: read-only review projection only; authoritative plan stays in `InstallFlow`
- proposed Lab consumer: `lab/scenarios/mod-it` ChangeStaging dock (simulated plan)

### VisualSnapshot

- current source DTOs: snapshot list/diff — `SnapshotDiff`, `SnapshotDiffEntry`, `LockfileDriftReport` (commands in tauri.ts, e.g. list/diff/restore)
- current UI: snapshots section in `pages/InstanceEditor.tsx` (Snapshots/Loadouts), `components/installed-content/*`
- proposed live adapter: render role/scope/world boundary; backend remains authority for restore
- proposed Lab consumer: `lab/scenarios/undo-it` `RecoveryTimeline`

### VisualCrashEvidence

- current source DTOs: `CrashReportInfo`, `CrashTriageResult`, `CrashEvidenceSource`, `EvidenceSourceKind`, evidence/suspects/outcome
- current UI: `components/CrashInvestigator.tsx`
- proposed live adapter: convert evidence into hypotheses + bounded summaries; no full logs by default
- proposed Lab consumer: `lab/scenarios/fix-it` `CrashEvidenceBoard` (later batch)

### VisualRuntimeState

- current source DTOs: Java inspection + memory recommendation (runtime inspection commands + memory recommendation; JVM args remain backend-side)
- current UI: Instance Editor -> Java section, `pages/settings/*`, Advanced Mode settings
- proposed live adapter: keep current/recommended/proposed distinct; friendly units; no raw JVM flags
- proposed Lab consumer: `lab/scenarios/heal-it` RuntimeWorkbench (later batch)

### VisualNetworkReadiness

- current source DTOs: **none** — no truthful read-only aggregate offline-readiness command exists (SOL-0 §11 / SAFETY_BOUNDARIES §11)
- current UI: `components/offline-banner.tsx`, Privacy settings (`pages/Privacy.tsx`), network policy in `agora-core`
- proposed live adapter: requires future core-owned read-only query; `unknown` required otherwise
- proposed Lab consumer: `lab/scenarios/take-it-offline` `NetworkReadinessMap` (later batch; simulation only)

## Architecture contradictions / notes (flagged, not silently changed)

- None found so far. Guide topic IDs referenced by `LESSON_MAP.md` (`instances`, `modding-foundations`, `install-update`, `launching`, `java-performance`, `crash-recovery`, `snapshots-loadouts`, `privacy-offline`) all exist in `desktop/src/data/guideContent.ts` `GUIDE_TOPICS`.
- `visual/`, `domain/`, `lab/` must not import `@tauri-apps/*`, `lib/tauri.ts`, `live/`, or current operation components — enforced by automated import check (DEEPSEEK-2).
- Reduced motion: global CSS in `desktop/src/index.css` (`:root[data-motion="reduced"]` and `@media (prefers-reduced-motion: reduce)`) already neutralizes CSS animation/transitions app-wide; `useUiPreferences().preferences.motion` gives the explicit preference for JS-side behavior.

## How to launch/test (vertical slice)

```powershell
cd desktop
npm run build          # interactive boundary check + tsc + vite build
npm run test:unit      # vitest run — 114 tests across 14 files
npm run dev            # Vite dev server; open the "Agora Lab" sidebar tab
npx playwright test    # e2e — 241 tests (run when asked / needed)
```

## What the vertical slice ships

- `desktop/src/features/interactive/domain/` — `models.ts`, `intents.ts`, `state.ts`, `guards.ts` (pure; no React/Tauri/app imports).
- `desktop/src/features/interactive/visual/` — `primitives/` (stateMarks, statusChips, LinearView, announce, useReducedMotion) + shared visuals `InstanceBench`, `ContentGraph`, `ChangeStaging`, `RecoveryTimeline` (controlled; emit `VisualIntent` only).
- `desktop/src/features/interactive/lab/` — `scenarioTypes.ts`, `lessonEngine.ts`, `simulationAdapter.ts`, `progressStore.ts`, `LabShell.tsx`, `ScenarioView.tsx`, `scenarios/{buildIt,modIt,undoIt}.ts`.
- Seams touched: `App.tsx` (new "Agora Lab" tab at `BASE_TABS[5]`, renders `LabShell`, Guide deep-link state, `handleOpenGuide`/`handleNavigateStandard`), `lib/useDestination.ts` (`'lab'` added to `Tab`), `pages/Guide.tsx` (optional `initialTopicId` deep-link prop), `package.json` (vitest + `check:boundaries`), `vite.config.ts` (test section), `scripts/check-interactive-boundaries.mjs`, `src/test/setup.ts` (jsdom matchMedia + in-memory localStorage polyfill for Node 25's broken global).
- Lab progress: `window.localStorage` key `agora-lab-progress`, versioned, non-sensitive, lesson-version mismatch resets safely.
- Zero real mutation: Lab has no Tauri/live/operation imports (enforced by script + test); no live adapters were built (that is DEEPSEEK-6); High Interaction Mode was NOT implemented.

## Handoff (DEEPSEEK-5 remaining-adventures batch)

```text
Agent: DeepSeek V4 Flash
Phase: DEEPSEEK-5 — remaining Lab adventures (Heal It, Fix It, Take It Offline) complete
Commit / branch / dirty status: master; uncommitted (docs/interactive/ untracked from SOL-0)
Files changed (this batch):
  desktop/src/features/interactive/visual/HealthLens.tsx           (new)
  desktop/src/features/interactive/visual/LoaderRail.tsx           (new)
  desktop/src/features/interactive/visual/RuntimeWorkbench.tsx     (new)
  desktop/src/features/interactive/visual/CrashEvidenceBoard.tsx   (new)
  desktop/src/features/interactive/visual/NetworkReadinessMap.tsx  (new)
  desktop/src/features/interactive/lab/scenarios/healIt.ts         (new)
  desktop/src/features/interactive/lab/scenarios/fixIt.ts          (new)
  desktop/src/features/interactive/lab/scenarios/takeItOffline.ts  (new)
  desktop/src/features/interactive/lab/simulationAdapter.ts        (register 6 adventures)
  desktop/src/features/interactive/lab/ScenarioView.tsx            (heal/fix/offline views; labCapabilities)
  desktop/src/features/interactive/lab/scenariosRemaining.test.ts  (new: reducers + intent mapping)
  desktop/src/features/interactive/visual/newVisuals.test.tsx      (new: visual smoke + intent tests)
  desktop/src/features/interactive/lab/LabShell.test.tsx           (+ 3 end-to-end playthroughs)
  desktop/src/features/interactive/lab/simulationAdapter.test.ts   (6-scenario contract coverage)
  docs/interactive/IMPLEMENTATION_STATUS.md                        (this log + handoff)
Tests run:
  npm run build — allowlist boundary check OK (45 files), tsc clean, vite build succeeds
    (pre-existing >500kB chunk warning only; bundle grew ~39 kB gzip — SAFE DEBT 3 lazy-loading note applies)
  node scripts/check-interactive-boundaries.mjs --root scripts/boundary-fixtures/interactive --fixtures
    — OK: every fixture file produced a violation (11 total)
  npm run test:unit — 114/114 pass (14 files)
  npx playwright test — 241/241 pass (full e2e regression)
How to launch/test: cd desktop && npm run dev -> "Agora Lab" tab. The selection screen now lists six
  adventures (Build, Mod, Heal, Fix, Undo, Offline). Each new adventure is a full simulated playthrough:
  Heal It (scan -> loader -> Java+memory -> revalidate), Fix It (evidence -> hypothesis -> recoverable
  experiment -> confirm/restore), Take It Offline (inspect -> prepare content -> launch mode -> recheck).
Known failures: none. Carried forward (non-blocking): Terra P3 wordmark polish; SOL-1 FIX BEFORE LIVE
  MODE + SAFE DEBT lists (incl. route-level lazy loading of the Lab). Live adapters / High Interaction /
  live mutation intentionally absent (SOL-2 gate).
Decisions made (this batch):
  - Three adventures added with the same deterministic reducer / namespaced-id / decision-gate contract.
  - Heal It teaches severity hierarchy and proven-vs-indeterminate loader; a kept Java warning persists.
  - Fix It treats suspects as hypotheses, serious-confirms the one-change experiment, and never claims
    one launch proves a cause; cancelling restores from the recovery point.
  - Take It Offline is simulation-only and never turns Unknown into Ready; direct launch is
    blocked-by-policy for sign-in (per SOL-0 offline-readiness boundary).
  - Shared visuals stay controlled: only review-loader / propose-memory / review-offline-readiness /
    review-health intents are emitted; no operation callbacks.
Decisions explicitly not made: live read adapters; High Interaction Mode; any live mutation;
  offline-readiness backend query; optional Memory mini-lab (not required); FIX BEFORE LIVE MODE items.
Required next agent: Luna for visual regression (screenshots/layout/smoke on the six adventures, per the
  coordination plan's "Luna regression after each batch"), then Terra for the deep UX review of the new
  adventures (TERRA-5), then Sol for SOL-3 educational correctness.
Why work is stopping: the coordination plan hands each completed adventure batch to Luna for regression
  before Terra's UX review; no live work may proceed before SOL-2.
```

## Handoff (SOL-2 gate fixes — batch 1; DEEPSEEK-6 remediation)

```text
Agent: DeepSeek V4 Flash
Phase: SOL-2 gate fixes — batch 1 (DEEPSEEK-6 remediation) — five cross-cutting blockers + §15.10 corrections complete (re-review requested)
Commit / branch / dirty status: master; uncommitted (docs/interactive/ untracked from SOL-0)
Files changed (this batch):
  desktop/src/features/interactive/live/liveScene.ts           (Fragment per read; derivedFreshness;
                                                                error instance -> empty, host error)
  desktop/src/features/interactive/live/LiveInteractiveHost.tsx (latest-wins refresh, keep-scene-visible
                                                                while refreshing, canonical processState +
                                                                installActive props, controller routing)
  desktop/src/features/interactive/live/LiveSceneView.tsx     (per-fragment availability rendering,
                                                                honest read-only copy, InstanceBench status)
  desktop/src/features/interactive/live/intentController.ts   (NEW: 8-gate routing + approved bridges +
                                                                operation seams)
  desktop/src/features/interactive/live/readAdapters/index.ts (processUnknown, healthKnown, availability
                                                                params; conservative unknowns)
  desktop/src/features/interactive/visual/HealthLens.tsx      (unavailable state + capability-gated Review)
  desktop/src/features/interactive/visual/RuntimeWorkbench.tsx(canProposeMemory gate + reason)
  desktop/src/features/interactive/visual/RecoveryTimeline.tsx(canPreviewSnapshot gate)
  desktop/src/features/interactive/visual/CrashEvidenceBoard.tsx (canOpenCrashDoctor Open Crash Doctor)
  desktop/src/features/interactive/domain/guards.ts           (isIntentEnabled helper)
  desktop/src/pages/InstanceEditor.tsx                        (wire presentationPreference + processState +
                                                                installActive)
  desktop/scripts/check-interactive-boundaries.mjs            (live subarea boundary: read/core/bridges,
                                                                tauri READ-command allowlist, per-specifier
                                                                type-only skip)
  desktop/scripts/boundary-fixtures/interactive/live/**       (fixture-mutation, fixture-unknown)
  desktop/src/features/interactive/live/*.test.ts(x)          (intentController, liveScene fragments,
                                                                updated host/boundary)
  desktop/src/features/interactive/visual/*.test.ts(x)        (capability-gated visual tests)
  docs/interactive/IMPLEMENTATION_STATUS.md                   (this log + handoff)
Tests run:
  npm run build — live-subarea boundary OK (60 files), tsc clean, vite build succeeds
  node scripts/check-interactive-boundaries.mjs --root scripts/boundary-fixtures/interactive --fixtures
    — OK: every fixture file produced a violation (13 total, incl. live read-mutation + unclassified live)
  npm run test:unit — 152/152 pass (21 files)
  npx playwright test — 241/241 pass (full e2e regression)
How to launch/test: cd desktop && npm run dev. My Instances -> instance -> "High Interaction view".
  In the Tauri app the read-only surface loads real data with per-fragment availability; a failed read
  shows unavailable/unknown (never ready). Refresh keeps the scene visible (latest-wins). Review intents
  route through routeLiveIntent to the Standard surface. SOL-2 re-review: npm run check:boundaries,
  test:unit, build; live/intentController.test.ts + live/liveScene.test.ts + host tests.
Known failures: none. Rejected seams (disable, health repair, snapshot restore, crash experiments,
  memory) are NOT implemented. Approved bridges are disabled scaffolds behind off capabilities.
  Carried forward (non-blocking): Terra P3 wordmark polish; SAFE DEBT list (route-level lazy loading);
  offline-readiness aggregate query absent (Take It Offline stays simulation-only).
Decisions made (this batch):
  - Every live read is a Fragment (ok/error); aggregate freshness is derived (any error -> unknown).
    Failed health/snapshots/runtime render as unavailable, never ready/empty.
  - One live intent controller routes every intent (source -> capability -> freshness -> availability ->
    approved bridge); rejected seams are capability-blocked; refresh-required for non-fresh scenes.
  - Shared visuals gate every operation-shaped command by capability; the controller enforces the same.
  - Refresh is latest-wins and keeps the last scene visible; canonical process/install state is consumed.
  - live/ is enforced by subarea: read layer (read-only tauri allowlist), core (types only), bridges
    (Standard controllers only); unknown live files and mutation imports fail the build.
  - presentationPreference is wired (versioned, default standard).
Decisions explicitly not made: NO live mutation or real High Interaction action enabled (SOL-2 must
  verify the bridges first); no offline-readiness backend query; no Terra TERRA-5 / SOL-3 work.
Required next agent: Sol for the SOL-2 re-review (checklist per §15.11: fragment truthfulness, latest-wins
  controller + canonical state, capability enforcement, live subarea/import enforcement, disabled
  approved-bridge scaffolds, rejected seams absent, corrected handoff claims, baseline gates).
Why work is stopping: per the SOL-2 gate, DeepSeek fixes the blockers and returns to Sol; real live
  actions are not enabled until Sol verifies the bridges.
```

## Handoff (SOL-2 gate fixes — batch 2; DEEPSEEK-6 remediation)

```text
Agent: DeepSeek V4 Flash
Phase: SOL-2 gate fixes — batch 2 (DEEPSEEK-6 remediation, §16) — BLOCKERs A-D + 16.6/16.7 corrections complete; re-review requested
Commit / branch / dirty status: master; uncommitted (docs/interactive/ untracked from SOL-0)
Files changed (this batch):
  desktop/src/features/interactive/live/LiveInteractiveHost.tsx  (REWRITE: monotonic request generation
                                                                never reset, instanceId verification,
                                                                retained scene source marked refreshing
                                                                while in flight, accepted read clears
                                                                refreshing, isolated instance-change vs
                                                                canonical effects, all canonical phases
                                                                projected, controller conflict busy,
                                                                forwards {bridge, context})
  desktop/src/features/interactive/live/operationBridges/index.ts (NEW: BridgeContext union, LiveReviewRoute,
                                                                StandardBridgeHandlers, openBridge dispatch)
  desktop/src/features/interactive/live/intentController.ts      (routeLiveIntent(scene, intent, caps,
                                                                instanceId, conflict) -> review route with
                                                                context; contextForBridge; ApprovedBridge)
  desktop/src/features/interactive/live/liveCapabilities.ts     (EVERY consequential capability -> false)
  desktop/src/features/interactive/domain/guards.ts             (capabilityGate handles review-staged-changes
                                                                via canProposeInstall)
  desktop/src/pages/InstanceEditor.tsx                          (StandardBridgeHandlers via openBridge: fresh
                                                                checkInstanceHealth->onReviewHealth,
                                                                openLoaderChooser, detectDrift+setSnapshotDiff
                                                                +setActiveTab, onInvestigate,
                                                                onOpenBrowseForInstance)
  desktop/src/features/interactive/live/LiveSceneView.tsx        (availability note for unavailable runtime)
  desktop/src/features/interactive/visual/RuntimeWorkbench.tsx  (unavailable guard; "GB configured")
  desktop/src/features/interactive/visual/HealthLens.tsx        ("No findings — this instance is ready to
                                                                launch.")
  desktop/src/features/interactive/visual/InstanceBench.tsx     (data-testid + launch/lock state attrs for
                                                                deterministic phase tests)
  desktop/src/features/interactive/live/readAdapters/index.ts   (contentToVisual O(nodes+relationships);
                                                                crashToVisual recoveryReady:false)
  desktop/src/features/interactive/live/LiveInteractiveHost.test.tsx (overlapping refresh, instance switch,
                                                                in-flight review, all phases, install
                                                                conflict, bridge+context forwarding)
  desktop/src/features/interactive/live/liveDefaultLoader.test.ts (NEW: mocks @/lib/tauri; individual +
                                                                total default-command failures through
                                                                readLiveData->defaultLiveLoad->buildHostData)
  desktop/src/features/interactive/live/intentController.test.ts (routeLiveIntent new signature + context;
                                                                default caps block every consequential intent)
  desktop/src/features/interactive/live/boundary.test.ts        (resolve live sibling imports against
                                                                live/domain/visual containment)
  desktop/scripts/check-interactive-boundaries.mjs              (resolve EVERY live specifier; edge direction
                                                                per subarea; tauriImportForm for
                                                                namespace/side-effect/dynamic/reexport-star/
                                                                import-equals; freshness.ts -> read)
  desktop/scripts/boundary-fixtures/interactive/live/**         (fixture-install, -disable, -launch, -settings,
                                                                -relative-bypass, -dynamic, -reexport,
                                                                operationBridges/fixture-tauri)
  docs/interactive/IMPLEMENTATION_STATUS.md                     (this log + corrected DEEPSEEK-7 claims)
Tests run:
  npm run build — production boundary OK (62 files), tsc clean, vite build succeeds
  node scripts/check-interactive-boundaries.mjs --root scripts/boundary-fixtures/interactive --fixtures
    — OK: every fixture file produced a violation (21 total)
  npm run test:unit — 167/167 pass (22 files)
  npx playwright test — 241/241 pass (full e2e regression)
How to launch/test: cd desktop && npm run dev. My Instances -> instance -> "High Interaction view".
  In the Tauri app the read-only surface loads real data with per-fragment availability. With default
  (shipped) capabilities every consequential review action is OFF, so no review control is reachable.
  Unit tests exercise the controller/bridges with a TEST-ONLY capability set; the shipped host default
  stays all-false. SOL-2 re-review: npm run check:boundaries; node scripts/check-interactive-boundaries.mjs
  --root scripts/boundary-fixtures/interactive --fixtures; npm run test:unit; npm run build;
  live/intentController.test.ts, live/LiveInteractiveHost.test.tsx, live/liveDefaultLoader.test.ts.
Known failures: none. All consequential live capabilities are OFF. Rejected seams (disable, health
  repair, snapshot restore, direct crash experiments, memory) have NO bridge. Carried forward
  (non-blocking): Terra P3 wordmark polish; SAFE DEBT list (route-level lazy loading); offline-readiness
  aggregate query absent (Take It Offline stays simulation-only).
Decisions made (this batch):
  - Refresh: retained scene source is marked refreshing while a read is in flight (non-executable);
    the accepted read installs a new revision and clears refreshing. One monotonic request generation
    that is never reset; the requested instance id is verified before applying results.
  - Instance-change reset is isolated from process/install-state updates; all canonical phases
    (launching/running/stopping/delegated/failed) are projected; canonical conflict feeds the
    controller's availability gate.
  - Every consequential live capability defaults to false; only a TEST-ONLY capability set enables the
    controller/bridge routing tests. No bridge exists for a rejected seam.
  - Narrow contextual bridges for the approved seams only: health-review (fresh checkInstanceHealth ->
    HealthDialog), loader-review (openLoaderChooser), snapshot-compare (detectDrift + setSnapshotDiff +
    snapshots tab), crash-doctor (onInvestigate), install-flow (onOpenBrowseForInstance). The host
    forwards { bridge, context }; each bridge re-reads/re-resolves on the Standard surface.
  - Live boundary now resolves every local specifier (no ./ or ../ escape), enforces per-subarea edge
    direction, and classifies namespace/side-effect/dynamic/star-reexport/import-equals tauri imports as
    unverifiable violations (never type-only). Fixture coverage raised to 21 files.
  - Runtime is an err fragment when detail, memory, or javas fail; crash recoveryReady is false at read
    time (Crash Doctor creates the recovery point before any experiment).
  - Corrected the DEEPSEEK-7 handoff claim: before this batch the health/loader/crash/snapshot review
    controls WERE reachable (four capabilities were true); they are now all off.
Decisions explicitly not made: NO live mutation or real High Interaction action enabled; no enablement of
  health/loader/snapshot/crash/install/remove capabilities; no approval of dependency disable, direct
  repair, restore, direct crash experiment, or memory mutation; no offline-readiness backend query; no
  Terra TERRA-5 / Luna / SOL-3 work.
Required next agent: Sol for the second SOL-2 bounded re-review (checklist per §16.9: BLOCKERs 16.2-16.6
  closed, every consequential capability disabled, corrected DEEPSEEK-7 handoff claims, baseline gates).
  NOTE: the plan's DEEPSEEK-7 (real High Interaction operations) remains the next real phase AFTER this
  gate passes — the bridges above are staged and dormant, not enabled.
Why work is stopping: per the SOL-2 gate, DeepSeek fixes the blockers and returns to Sol; real live
  actions are not enabled until Sol verifies the contextual bridges.
```

## Handoff (SOL-2 gate fixes — batch 3; DEEPSEEK-6 remediation)

```text
Agent: DeepSeek V4 Flash
Phase: SOL-2 gate fixes — batch 3 (DEEPSEEK-6 remediation, §17) — closes the second bounded re-review
  BLOCKERs A-C + §17.6; 3rd re-review requested
Commit / branch / dirty status: master; uncommitted (docs/interactive/ untracked from SOL-0)
Files changed (this batch):
  desktop/src/features/interactive/live/LiveInteractiveHost.tsx  (REWRITE: stores the BASE unprojected
                                                                scene; projectCanonical() at render —
                                                                reversible busy (active->idle clears),
                                                                latest-wins canonical (a change during an
                                                                unresolved read is applied on accept);
                                                                review dispatch records an in-review
                                                                proposal (duplicate gate coalesces) cleared
                                                                by the next accepted read (terminal
                                                                refresh); install-flow context enriched with
                                                                backend-derived filename/contentKind)
  desktop/src/features/interactive/live/operationBridges/index.ts (REWRITE: BridgeContext install-flow is
                                                                action-bearing (install|update|remove|
                                                                review) + contentId; LiveReviewRoute is a
                                                                DISCRIMINATED union; openBridge passes the
                                                                full typed context; installActionLabel)
  desktop/src/features/interactive/live/intentController.ts     (RouteResult review arm now returns a
                                                                discriminated LiveReviewRoute via
                                                                reviewRouteFor(); contextForBridge retains
                                                                action + contentId per intent)
  desktop/src/pages/InstanceEditor.tsx                          (openInstallFlow builds an ACTION-BEARING
                                                                InstallIntent and opens the canonical
                                                                InstallFlow: remove (fresh-verified filename),
                                                                install (curated itemId from manifest),
                                                                update (mod's Standard install review for
                                                                target version), unresolvable -> explicit
                                                                error (never silent Browse);
                                                                installActive={packInstall?.status==='running'})
  desktop/scripts/check-interactive-boundaries.mjs              (tauriImportForm -> tauriImportForms:
                                                                AGGREGATES every matching form; rejects when
                                                                ANY form is prohibited/unverifiable; named
                                                                imports use ORIGINAL names
                                                                propertyName??name — alias-safe)
  desktop/scripts/boundary-fixtures/interactive/live/readAdapters/
    fixture-alias-launder.ts (NEW)                               (restoreSnapshot as getInstanceDetail ->
                                                                flagged as restoreSnapshot)
    fixture-mixed.ts (NEW)                                       (prohibited import followed by safe import
                                                                from same specifier -> still flagged)
  desktop/src/features/interactive/live/LiveInteractiveHost.test.tsx (+6: idle->launching->idle clears busy,
                                                                install active->inactive, canonical change
                                                                during unresolved load applies latest, two
                                                                genuinely overlapping refreshes newest-wins,
                                                                in-flight coalescing + refresh clears,
                                                                install-flow enrichment)
  desktop/src/features/interactive/live/operationBridges.test.ts (NEW: every dormant adapter dispatches to
                                                                the correct handler with full context)
  desktop/src/features/interactive/live/intentController.test.ts (+ action-bearing install-flow routing)
  desktop/src/features/interactive/live/liveDefaultLoader.test.ts (+ queryLaunchState individual failure)
  docs/interactive/IMPLEMENTATION_STATUS.md                     (this log + handoff)
Tests run:
  npm run build — production boundary OK (62 files), tsc clean, vite build succeeds
  node scripts/check-interactive-boundaries.mjs --root scripts/boundary-fixtures/interactive --fixtures
    — OK: every fixture file produced a violation (24 total, incl. alias + mixed-import cases)
  npm run test:unit — 182/182 pass (23 files)
  npx playwright test — 241/241 pass (full e2e regression)
How to launch/test: cd desktop && npm run dev. My Instances -> instance -> "High Interaction view".
  With default (shipped) capabilities every consequential review action is OFF, so no review control is
  reachable. Unit tests exercise the controller/bridges with a TEST-ONLY capability set; the shipped
  host default stays all-false. Sol 3rd re-review: npm run check:boundaries; fixture mode; npm run
  test:unit; npm run build; live/operationBridges.test.ts, live/LiveInteractiveHost.test.tsx
  (canonical transitions, overlapping refresh, in-flight lifecycle), live/intentController.test.ts,
  live/liveDefaultLoader.test.ts, scripts/check-interactive-boundaries.mjs fixture mode.
Known failures: none. All consequential live capabilities are OFF. Rejected seams (disable, health
  repair, snapshot restore, direct crash experiments, memory) have NO bridge. Carried forward
  (non-blocking): Terra P3 wordmark polish; SAFE DEBT list (route-level lazy loading); offline-readiness
  aggregate query absent (Take It Offline stays simulation-only); health classification still scans
  findings per node (dependency gestures remain disabled; the visual does not claim a complete graph).
Decisions made (this batch):
  - Canonical state is projected at RENDER over the base unprojected read scene: busy is derived only
    from canonical state (reversible — idle->launching->idle clears busy) and always uses the LATEST
    canonical values (no stale acceptance-time closures; a canonical change during an unresolved read is
    applied on accept). Only a RUNNING install task counts as active.
  - Install-flow routes are action-bearing and discriminated: the host enriches the context with the
    backend-derived filename/contentKind from the accepted live scene (never parsed from a visual id);
    InstanceEditor constructs a real InstallIntent and opens the canonical InstallFlow (remove/install
    complete; update re-selects the target version in the Standard mod review; unresolvable content
    produces an explicit error instead of a silent Browse).
  - Review lifecycle is modeled at the host: an in-review proposal coalesces a second review via the
    controller's duplicate gate; the next accepted read (terminal refresh) clears it. Backend outcomes
    stay in the Standard surface.
  - The Tauri allowlist aggregates every matching import form and evaluates ORIGINAL import names, so
    alias laundering and mixed prohibited/safe imports can no longer bypass it. Fixtures: 24/24.
Decisions explicitly not made: NO live mutation or real High Interaction action enabled; no enablement of
  health/loader/snapshot/crash/install/remove capabilities; no approval of dependency disable, direct
  repair, restore, direct crash experiment, or memory mutation; no offline-readiness backend query; no
  Terra TERRA-5 / Luna / SOL-3 work.
Required next agent: Sol for the third SOL-2 bounded re-review (checklist per §17.9: reversible/latest-wins
  canonical state incl. transition + overlap tests, action-bearing InstallFlow bridge + discriminated
  routes + in-flight/terminal-refresh lifecycle + every-adapter tests, aggregated alias-safe Tauri
  allowlist + alias/mixed fixtures, queryLaunchState default-failure coverage, every consequential
  capability disabled). NOTE: the plan's DEEPSEEK-7 (real High Interaction operations) remains the next
  real phase AFTER this gate passes.
Why work is stopping: per the SOL-2 gate, DeepSeek fixes the blockers and returns to Sol; real live
  actions are not enabled until Sol verifies the contextual bridges.
```

## Handoff (SOL-2 gate fixes — batch 4; DEEPSEEK-6 remediation + Rust backend)

```text
Agent: DeepSeek V4 Flash
Phase: SOL-2 gate fixes — batch 4 (DEEPSEEK-6 remediation + Rust backend, §18) — closes the third
  bounded re-review BLOCKERs A-D; 4th re-review requested
Commit / branch / dirty status: master; uncommitted (docs/interactive/ untracked from SOL-0)
Files changed (this batch):
  desktop/src/features/interactive/live/intentController.ts   (routeLiveIntent now takes TYPED
                                                                AvailabilityInput {locked, recoveryBusy,
                                                                processBusy, installBusy} with a distinct
                                                                explanation per condition; selection/
                                                                inspection stay available)
  desktop/src/features/interactive/live/LiveInteractiveHost.tsx (derives AvailabilityInput from the
                                                                projected scene + canonical state; in-review
                                                                marker survives manual refresh via
                                                                reviewInFlightRef and is cleared only by
                                                                unmount/instance change; re-asserted on each
                                                                accepted read)
  desktop/src/pages/InstanceEditor.tsx                          (health + crash bridges now LEAVE High
                                                                Interaction first — every bridge is option (a)
                                                                Standard navigation; health is ordinary
                                                                Standard review)
  desktop/src/components/HealthDialog.tsx                       (reviewOnly: handleFixDisable returns early
                                                                (never disableModForTest) and Disable buttons
                                                                render only when !reviewOnly)
  desktop/src/components/HealthDialog.reviewOnly.test.tsx       (NEW: reviewOnly hides Disable + never calls
                                                                disableModForTest; non-review keeps it)
  desktop/e2e/health-launch.spec.ts                             (review-only dialog test updated: asserts NO
                                                                Disable + zero disable_mod_for_test calls;
                                                                Standard launch-path disable test unchanged)
  desktop/src-tauri/src/commands.rs                             (NEW ensure_install_apply_allowed: rejects
                                                                running target / launch reservation /
                                                                competing install under the SAME state lock
                                                                as registration; apply_install_plan uses it +
                                                                4 new Rust integration tests)
  desktop/src/features/interactive/live/intentController.test.ts (+ per-state availability reasons +
                                                                selection-still-available)
  desktop/src/features/interactive/live/LiveInteractiveHost.test.tsx (manual refresh does NOT clear the
                                                                in-flight marker; remount = fresh read + no
                                                                stale state; locked/recovery block review)
Tests run:
  npm run build — production boundary OK (63 files), tsc clean, vite build succeeds
  node scripts/check-interactive-boundaries.mjs --root scripts/boundary-fixtures/interactive --fixtures
    — OK: every fixture file produced a violation (24 total)
  npm run test:unit — 187/187 pass (24 files)
  cargo test (desktop/src-tauri) — 68/68 pass (incl. 4 new install-exclusion tests)
  npx playwright test — 241/241 pass (full e2e regression)
How to launch/test: cd desktop && npm run dev. My Instances -> instance -> "High Interaction view".
  With default (shipped) capabilities every consequential review action is OFF, so no review control is
  reachable. Unit tests exercise the controller/bridges with a TEST-ONLY capability set; the shipped
  host default stays all-false. Sol 4th re-review: npm run check:boundaries; fixture mode; npm run
  test:unit; cargo test; npm run build; live/intentController.test.ts (availability states),
  live/LiveInteractiveHost.test.tsx (locked/recovery, refresh-preserves-marker, remount-fresh-read),
  components/HealthDialog.reviewOnly.test.tsx, commands.rs ensure_install_apply_allowed tests.
Known failures: none. All consequential live capabilities are OFF. Rejected seams (disable, health
  repair, snapshot restore, direct crash experiments, memory) have NO bridge; the Standard HealthDialog
  retains its disable repair only outside reviewOnly (Standard surface). Carried forward (non-blocking):
  Terra P3 wordmark polish; SAFE DEBT list (route-level lazy loading); offline-readiness aggregate query
  absent; no real current install/update gesture has a fresh curated candidate/target-version source
  (Sol requires one before enabling install/update — not built in this batch).
Decisions made (this batch):
  - Availability is a typed readiness/lock input, not a boolean: player locks, pending/failed recovery,
    active process/launch, and active installs each block review with their own explanation.
  - Terminal lifecycle = option (a): every bridge leaves High Interaction before Standard work; re-entry
    remounts the host and performs a fresh read, so no stale live state survives any Standard outcome.
    A manual refresh is NOT a review terminal event and does not clear the in-flight marker.
  - The High Interaction health route is ordinary Standard navigation (reviewOnly HealthDialog), and
    reviewOnly now hides every rejected repair/disable control and can never call disableModForTest.
  - apply_install_plan atomically rejects a running target process and launch reservation under the
    same state lock before registering the install, retaining one-active-install behavior.
Decisions explicitly not made: NO live mutation or real High Interaction action enabled; no enablement of
  health/loader/snapshot/crash/install/remove capabilities; no approval of dependency disable, direct
  repair, restore, direct crash experiment, or memory mutation; no fresh curated candidate/target-version
  source for install/update; no offline-readiness backend query; no Terra TERRA-5 / Luna / SOL-3 work.
Required next agent: Sol for the fourth SOL-2 bounded re-review (checklist per §18.9: typed availability
  gate with lock/recovery/process/install states + tests, owned terminal-refresh lifecycle via leave-HI
  + fresh-read-on-return + manual-refresh-not-terminal, reviewOnly no-disable + no disableModForTest,
  atomic install launch/process exclusion + Rust tests, every consequential capability disabled). NOTE:
  the plan's DEEPSEEK-7 (real High Interaction operations) remains the next real phase AFTER this gate
  passes, and install/update additionally require a fresh curated candidate/target-version source.
Why work is stopping: per the SOL-2 gate, DeepSeek fixes the blockers and returns to Sol; real live
  actions are not enabled until Sol verifies the contextual bridges.
```

## Handoff (SOL-2 gate fixes — batch 5; DEEPSEEK-6 remediation + Rust backend)

```text
Agent: DeepSeek V4 Flash
Phase: SOL-2 gate fixes — batch 5 (DEEPSEEK-6 remediation + Rust backend, §19) — closes the fourth
  bounded re-review BLOCKERs A-B; 5th re-review requested
Commit / branch / dirty status: master; uncommitted (docs/interactive/ untracked from SOL-0)
Files changed (this batch):
  crates/agora-core/src/state.rs                            (NEW target-aware active_launches: HashSet;
                                                              AppState::new initializes it)
  desktop/src-tauri/src/commands.rs                          (NEW ensure_launch_admitted shared helper used
                                                                by ALL THREE launch entries; delegated
                                                                launch_instance registers an atomic start
                                                                marker (cleared on failure + when
                                                                wait_delegated completes); direct +
                                                                recovery re-check running/reservation AND
                                                                admission under the SAME lock that sets
                                                                launch_reservation (closes preflight->
                                                                reservation race); ensure_install_apply_allowed
                                                                also rejects ERR_INSTALL_LAUNCH_ACTIVE;
                                                                +4 Rust tests: active-install->launch,
                                                                preflight->reservation gap, delegated marker
                                                                + cleanup/retry, mutual exclusion both ways)
  desktop/src/features/interactive/live/LiveInteractiveHost.tsx (LiveHostState.scene now carries the
                                                                instanceId it was loaded FOR; displayData is
                                                                withheld (loading, non-routable) whenever
                                                                state.instanceId !== instanceId — a RENDER
                                                                guard, never a passive reset)
  desktop/src/pages/InstanceEditor.tsx                        (openInstallFlow re-resolves per route with a
                                                                fresh getInstanceDetail(id) against THAT
                                                                manifest (never the retained page manifest);
                                                                new beginCanonicalOperationFor(targetId,
                                                                targetName, action); every rejection branch
                                                                leaves High Interaction first so no
                                                                in-review proposal is stranded)
  desktop/src/features/interactive/live/LiveInteractiveHost.test.tsx (+2: old scene withheld/
                                                                non-routable during an unresolved switch;
                                                                shared-filename staged-install routes to B,
                                                                never A data)
  docs/interactive/IMPLEMENTATION_STATUS.md                   (this log + handoff)
Tests run:
  npm run build — production boundary OK (63 files), tsc clean, vite build succeeds
  node scripts/check-interactive-boundaries.mjs --root scripts/boundary-fixtures/interactive --fixtures
    — OK: every fixture file produced a violation (24 total)
  npm run test:unit — 189/189 pass (24 files)
  cargo test (desktop/src-tauri) — 72/72 pass (incl. 4 new launch-exclusion tests)
  npx playwright test — 241/241 pass (full e2e regression)
How to launch/test: cd desktop && npm run dev. My Instances -> instance -> "High Interaction view".
  With default (shipped) capabilities every consequential review action is OFF, so no review control is
  reachable. Unit tests exercise the controller/bridges with a TEST-ONLY capability set; the shipped
  host default stays all-false. Sol 5th re-review: npm run check:boundaries; fixture mode; npm run
  test:unit; cargo test; npm run build; commands.rs ensure_launch_admitted + mutual-exclusion tests,
  live/LiveInteractiveHost.test.tsx (instance-identity withhold + no-A-to-B routing tests).
Known failures: none. All consequential live capabilities are OFF. Rejected seams (disable, health
  repair, snapshot restore, direct crash experiments, memory) have NO bridge. Carried forward
  (non-blocking): Terra P3 wordmark polish; SAFE DEBT list (route-level lazy loading); offline-readiness
  aggregate query absent; no real current install/update gesture has a fresh curated candidate/
  target-version source (Sol requires one before enabling install/update — not built in this batch).
Decisions made (this batch):
  - Launch/install mutual exclusion is now BOTH directions and target-aware: launches reject an active
    install at their final state-lock transition, and installs reject an active/reserved/marked launch.
    Delegated launches register an equivalent atomic start marker (no reservation exists) with failure
    and session-end cleanup; direct/recovery re-check under the reservation-setting lock.
  - Every accepted host scene is bound to the instance it was loaded for; a differing current
    instanceId withholds it as loading/non-routable at render time (React passive-effect timing is
    never a safety control). Old content can never be routed as the new instance.
  - The install/remove/update bridge re-resolves target + content with a fresh read per route (never the
    retained page manifest) and targets the explicit route instance; every rejection leaves High
    Interaction so no in-review proposal is stranded.
Decisions explicitly not made: NO live mutation or real High Interaction action enabled; no enablement of
  health/loader/snapshot/crash/install/remove capabilities; no approval of dependency disable, direct
  repair, restore, direct crash experiment, or memory mutation; no fresh curated candidate/target-version
  source for install/update; no offline-readiness backend query; no Terra TERRA-5 / Luna / SOL-3 work.
Required next agent: Sol for the fifth SOL-2 bounded re-review (checklist per §19.7: target-aware
  launch/install mutual exclusion in all three launch entries + command-level tests, scene bound to its
  loaded instance ID with a render-time withhold guard, per-route fresh re-resolve in the install bridge
  + no-stranded-proposal terminal rule, every consequential capability disabled). NOTE: the plan's
  DEEPSEEK-7 (real High Interaction operations) remains the next real phase AFTER this gate passes, and
  install/update additionally require a fresh curated candidate/target-version source.
Why work is stopping: per the SOL-2 gate, DeepSeek fixes the blockers and returns to Sol; real live
  actions are not enabled until Sol verifies the contextual bridges.
```

## Handoff (DEEPSEEK-7 — real High Interaction operations)

```text
Agent: DeepSeek V4 Flash
Phase: DEEPSEEK-7 — SOL-2 §20 APPROVED; approved High Interaction seams enabled + verified end-to-end
  (Terra TERRA-5 has since PASSED the live surface); LUNA smoke/regression pending
Commit / branch / dirty status: master; uncommitted (docs/interactive/ untracked from SOL-0)
Files changed (this batch):
  desktop/src/features/interactive/live/liveCapabilities.ts   (renamed liveHighInteractionCapabilities;
                                                                APPROVED seams ON — remove, health, loader,
                                                                snapshot compare, Crash Doctor; install/update
                                                                + rejected seams OFF)
  desktop/src/features/interactive/live/LiveInteractiveHost.tsx (import/default rename; honest doc comment)
  desktop/src/features/interactive/visual/HealthLens.tsx       (NEW "Review health" header button gated by
                                                                canReviewHealth — makes the approved health
                                                                inspection bridge reachable)
  desktop/src/features/interactive/live/LiveSceneView.tsx     (honest copy: "Reviews open the Standard
                                                                surface; content changes use the reviewed
                                                                InstallFlow.")
  desktop/src/features/interactive/live/intentController.ts   (unsupported reason no longer says "read-only")
  desktop/src/features/interactive/live/intentController.test.ts (shipped-defaults tests: approved seams
                                                                route to review; rejected/blocked seams off)
  desktop/src/features/interactive/live/LiveInteractiveHost.test.tsx (describe title updated)
  desktop/e2e/high-interaction.spec.ts                        (NEW: read surface + health dialog +
                                                                remove→InstallFlow end-to-end with a full
                                                                read-command mock)
  desktop/playwright.config.ts                                (reporter -> line + html(open:'never'); fixes
                                                                browser auto-open + Windows webServer teardown
                                                                hang that left the CLI alive after tests)
  docs/interactive/IMPLEMENTATION_STATUS.md                   (this log + handoff)
Tests run:
  npm run build — production boundary OK (63 files), tsc clean, vite build succeeds
  node scripts/check-interactive-boundaries.mjs --root scripts/boundary-fixtures/interactive --fixtures
    — OK: every fixture file produced a violation (24 total)
  npm run test:unit — 190/190 pass (24 files)
  npx playwright test — 243/243 pass (241 prior + 2 new High Interaction)
  (cargo test 72/72 unchanged this batch — Rust was last touched in batch 5)
How to launch/test: cd desktop && npm run dev; My Instances -> instance -> "High Interaction view".
  The approved seams are now live: Review health opens the Standard review-only HealthDialog; a loader
  finding's Review opens LoaderChooser; snapshot Compare shows the read-only drift; Open Crash Doctor
  navigates to CrashInvestigator; Stage removal opens the canonical InstallFlow (fresh per-route
  re-resolve). Install/update controls stay absent. Ensure a healthy Vite server is running before
  `npx playwright test` (Playwright reuses it; starting its own on Windows can hang teardown).
Known failures: none. Install/update gestures remain blocked (need a fresh curated candidate/
  target-version source, §19.5). Rejected seams (dependency disable, snapshot restore, direct health
  repair, direct crash experiments, memory) have NO bridge and their capabilities are OFF. Carried
  forward (non-blocking): Terra P3 wordmark polish; SAFE DEBT list (route-level lazy loading);
  offline-readiness aggregate query absent.
Decisions made (this batch):
  - Enabled exactly the SOL-2-APPROVED capabilities; renamed the factory for honesty (remove is no
    longer read-only); updated the live surface copy accordingly.
  - Added a reachable "Review health" control so the approved health-inspection bridge is actually
    reachable from the live surface (it opens the Standard review-only HealthDialog, leave-HI lifecycle).
  - Verified end-to-end with a new High Interaction e2e spec (real mock-backed instance data; health
    dialog; remove → InstallFlow resolve). Loader/snapshot/crash remain covered by the unit/integration
    bridge suites and are queued for Luna's visual regression.
  - Hardened the Playwright config so runs never auto-open a browser and always exit after tests.
Decisions explicitly not made: no enablement of install/update (blocked on the curated candidate/
  target-version source); no rejected-seam bridge; no offline-readiness backend query; no Terra TERRA-5
  / SOL-3 / DEEPSEEK-8 work.
Required next agent: LUNA for smoke/visual regression on the live High Interaction surface (screenshots/
  layout/state on the enabled bridges: health dialog, loader chooser, snapshot compare, Crash Doctor,
  remove→InstallFlow). NOTE: Terra TERRA-5 has already PASSED (2026-08-10, no P0/P1), so after Luna the
  next gate is Sol's SOL-3 educational correctness.
Why work is stopping: per the coordination plan, each completed batch hands to Luna for regression before
  the next review; Terra TERRA-5 has already passed, so the remaining sequence is LUNA smoke/regression →
  SOL-3 → TERRA final + LUNA final → DEEPSEEK final fixes → SOL-4. Install/update additionally await a
  fresh curated candidate/target-version source.
```
