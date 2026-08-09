# Interactive Experiences: Implementation Status

Status: DEEPSEEK-4 Terra-fix batch COMPLETE — awaiting Terra TERRA-4 retest, then SOL-1

Authoritative contracts: `MASTER_ARCHITECTURE.md`, `DOMAIN_MODELS.md`, `VISUAL_LANGUAGE.md`, `LESSON_MAP.md`, `SAFETY_BOUNDARIES.md` (all SOL-0, present).

## Phase log

| Phase | Agent | Status | Notes |
|---|---|---|---|
| SOL-0 architecture | Sol | Done | five contract docs under `docs/interactive/`; untracked in git |
| DEEPSEEK-1 repo mapping | DeepSeek | Done | this document |
| DEEPSEEK-2 visual framework | DeepSeek | Done | domain/ + visual/ + import-boundary check |
| DEEPSEEK-3 vertical slice | DeepSeek | Done (HARD STOP) | Build It, Mod It, Undo It + Lab shell + tests |
| Terra UX gate (TERRA-1..3) | Terra | Done | `UX_FINDINGS_TERRA.md`; verdict: do NOT proceed to SOL-1 until P1/P2 fixed |
| DEEPSEEK-4 Terra fixes | DeepSeek | Done | all 5 P1 + P2 fixed; tests green; see fix log below |
| Terra TERRA-4 retest | Terra | Not started | next |
| SOL-1 architecture gate | Sol | Not started | after Terra confirms |

## DEEPSEEK-4 fix log (Terra findings -> fixes)

| Finding | Fix |
|---|---|
| P1 Mod It post-apply still proposed/unchanged | `modIt.ts contentNodes` now projects applied result into `current` and clears all `proposed` markers; `ChangeStaging` gained an `outcome` prop rendering an applied outcome/return-point summary instead of "No changes staged"; `ModItView` passes the outcome. Covered by reducer + LabShell UI tests. |
| P1 generic confirm dialog called every danger a "restore" | `LabDecision` gained `confirmTitle`/`confirmBody`; `LabShell` confirm dialog now uses action-specific accessible name + copy. `replace-terrain-overhaul` names the removed/installed content and a new return point; only `confirm-restore` describes worlds/scope. |
| P1 Undo It reports restored but keeps bad current state | `undoIt.ts` confirm-restore now replaces `currentLabel` with "restored to …", appends an `undo-restore` snapshot to the timeline, and sets `restoredSummary` (worlds-included boundary); `UndoItView` renders a visible Restore outcome region. Pre/post states are visibly different. |
| P1 Build It isolation not visual | `InstanceBench` gained `roleLabel`/`statusLabel`/`highlight`; `BuildItView` renders "Your new instance" (highlighted) vs "Existing example · Separate · unchanged". |
| P1 Diagram view didn't teach relationships | `ContentGraph` added socket chips on node cards (filled/empty sockets, Requires/Recommends/Conflicts with, Missing/Blocking/Resolved text); Conflicts region shows only active conflicts; prose relationship list now lives in List view only. |
| P2 blocked choices only in a11y tree | `LabShell` tracks `lastAttempt`; blocked/caution feedback renders inline beside the attempted decision with destructive styling, in addition to the status region. |

Open non-gate item (from Terra P3): launcher wordmark truncates under High readability at 1366x768 — app-shell responsive polish, NOT a Lab gate blocker; deferred as follow-up.

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
npm run test:unit      # vitest run — 71 tests across 10 files
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

## Handoff (DEEPSEEK-4 Terra-fix batch)

```text
Agent: DeepSeek V4 Flash
Phase: DEEPSEEK-4 — Terra TERRA-1..3 P1/P2 fixes complete
Commit / branch / dirty status: master; uncommitted (docs/interactive/ untracked from SOL-0)
Files changed (this batch):
  desktop/src/features/interactive/lab/scenarios/modIt.ts      (post-apply current state, action-specific confirm)
  desktop/src/features/interactive/lab/scenarios/undoIt.ts     (visible post-restore state, restore-specific confirm)
  desktop/src/features/interactive/visual/ChangeStaging.tsx    (outcome prop / applied summary)
  desktop/src/features/interactive/visual/ContentGraph.tsx     (socket chips; conflicts region; list-only prose)
  desktop/src/features/interactive/visual/InstanceBench.tsx    (roleLabel/statusLabel/highlight)
  desktop/src/features/interactive/lab/ScenarioView.tsx        (outcome + restore-outcome rendering, role labels)
  desktop/src/features/interactive/lab/LabShell.tsx            (action-specific confirm; inline blocked feedback)
  desktop/src/features/interactive/lab/scenarioTypes.ts        (LabDecision.confirmTitle/confirmBody)
  desktop/src/features/interactive/**/*.test.ts(x)             (new/updated coverage)
  docs/interactive/IMPLEMENTATION_STATUS.md                    (this log + handoff)
  docs/interactive/UX_FINDINGS_TERRA.md                        (Terra's report; new)
Tests run:
  npm run build — boundary check OK (32 files), tsc clean, vite build succeeds
  npm run test:unit — 71/71 pass (added: post-apply current-state, post-restore visible state,
    socket chips, role labels, applied outcome, action-specific confirm, inline blocked feedback)
  npx playwright test — 241/241 pass (full e2e regression, incl. the earlier mod-detail-governance
    locator fix for the 'Agora Lab' sidebar tab)
How to launch/test: cd desktop && npm run dev -> "Agora Lab" tab. Retest each P1/P2 outcome:
  Mod It post-apply (Plan applied + fully current graph), Replace confirm dialog ('Confirm replacement'),
  Undo It restore outcome (restored current label + undo point + scope), Build It 'Your new instance' vs
  'Existing example', ContentGraph socket chips, blocked-choice inline feedback.
Known failures: none. Known limits unchanged: live adapters absent (DEEPSEEK-6); High Interaction Mode
  absent; Heal It / Fix It / Take It Offline / Memory not built; no live destructive wiring (SOL-2 pending).
  Terra P3 (sidebar wordmark truncation under High readability at 1366x768) is app-shell polish, NOT a Lab
  gate blocker — deferred as follow-up.
Decisions made (this batch):
  - Confirmation dialog copy is decision-owned (LabDecision.confirmTitle/confirmBody); shell only falls
    back to generic copy. Only a real restore may mention restoring worlds/scope.
  - Applied state clears proposed markers and projects into current (DOMAIN_MODELS committed-outcome rule).
  - Diagram view now carries socket chips (show, don't lecture); prose relationship list is the List view.
  - Instance benches are role-labelled ('Your new instance' / 'Existing example · Separate · unchanged').
  - Blocked/caution attempts render inline beside the action + destructive styling (P2).
Decisions explicitly not made:
  - Remaining adventures, live read adapters, High Interaction Mode, any live mutation, offline-readiness
    query, and P3 wordmark polish (deferred).
Required next agent: Terra for TERRA-4 retest + Golden-standard confirmation. If Terra is satisfied,
  then Sol for SOL-1 architecture scaling gate.
Why work is stopping: per coordination, DEEPSEEK hands back to Terra after fixing its UX findings;
  SOL-1 must not start until Terra confirms the P1 items on retest.
```
