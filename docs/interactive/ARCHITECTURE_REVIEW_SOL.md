# SOL-1 Architecture Scaling Review

Status: **BLOCKED — DeepSeek fixes required, then SOL-1 re-review**

Review date: 2026-08-09

Reviewer: Sol

Scope: the interactive domain and visual framework, Lab shell/lesson engine, Build It, Mod It, Undo It, DeepSeek’s Terra-fix batch, and Terra’s TERRA-4 retest

No product code was changed during this review.

## 1. Entry gate

The SOL-1 entry conditions are satisfied:

- the domain/visual/Lab framework exists under `desktop/src/features/interactive/`;
- Build It, Mod It, and Undo It are implemented;
- Terra recorded a TERRA-4 PASS after black-box retesting every prior P1/P2 finding;
- DeepSeek recorded and tested the corresponding fix batch;
- no live adapter, High Interaction Mode, or real mutation path has been added.

`IMPLEMENTATION_STATUS.md` lines 3 and 17 are stale and still say TERRA-4 is pending. The authoritative newer evidence is `UX_FINDINGS_TERRA.md` line 7 and its TERRA-4 handoff.

## 2. Verification performed

Current baseline:

```text
npm run check:boundaries  PASS — 32 interactive files checked
npm run test:unit         PASS — 71/71 tests in 10 files
npm run build             PASS — boundary check, TypeScript, and Vite production build
```

The Vite build retains the existing large-chunk warning; the main JavaScript artifact is 1,213.24 kB before gzip / 341.69 kB gzip in this worktree.

DeepSeek reports 241/241 Playwright tests passing, and Terra completed a fresh browser retest. SOL did not repeat the full Playwright suite because the findings below are static authority, alternate-input, and focus-management paths that the current tests do not cover.

The worktree remains uncommitted and contains the prior interactive implementation plus modified documentation-screenshot assets. Sol preserved all prior changes.

## 3. Executive decision

The current production imports contain no Tauri/backend operation dependency, no giant backend DTO copy, and no secret/authority fields. The lesson reducer is deterministic, the simulation source is explicit, current/proposed outcomes on Terra’s primary paths are corrected, and the visual vocabulary is promising.

However, the framework does not yet make its safety claims structurally true. A denylist-based boundary check can be bypassed by normal TypeScript imports; one graph gesture bypasses the serious-decision gate; the confirmation surface is not exclusive or focus-managed; and `ChangeStaging` exposes an operation-like callback outside the closed intent union.

These are scaling blockers because the next three adventures would build more behavior on the same authority and confirmation seams. **The remaining Lab adventures are not authorized yet.** No live or destructive wiring is authorized.

DeepSeek should fix only the BLOCKER items, add the specified regression tests, update the implementation handoff, and return to Sol for the SOL-1 re-review. FIX BEFORE LIVE MODE and SAFE DEBT items do not block that re-review unless a blocker fix makes them worse.

## 4. Gate matrix

| SOL-1 concern | Result | Summary |
|---|---|---|
| Simulation/live separation | **BLOCKED** | Current imports are clean, but the automated boundary is a bypassable denylist rather than a fail-closed structural boundary. |
| Backend DTO coupling | **PASS for current scope** | Domain models remain minimal and contain no Tauri DTO, path, fingerprint, receipt, token, or raw manifest. Live adapters do not exist yet. |
| Lesson-engine scalability | **CONDITIONAL** | Pure reducer and scenario contract are sound; central casts and checkpoint-only branch resume are safe debt. |
| Shared visual grammar | **BLOCKED** | Primary Terra routes are corrected, but an opaque review callback escapes the closed intent contract. Live uncertainty/availability semantics need later work. |
| Accessibility architecture | **BLOCKED** | Semantic controls and reduced-motion handling are good; the serious confirmation is not a real exclusive/focus-managed dialog. |
| Performance implications | **FIX BEFORE LIVE MODE / SAFE DEBT** | Current Lab scale is fine; ContentGraph is unbounded and superlinear, and the optional Lab bundle is eagerly loaded. |
| Accidental real-action paths | **BLOCKED** | One alternate visual intent bypasses `LabDecision.danger`; the boundary check and callback API would permit future authority leaks. |
| Terra conceptual findings | **PASS on retested paths** | All TERRA-1..3 P1/P2 findings are visibly corrected. The alternate graph path below was outside that retest route. |

## 5. BLOCKER findings

### BLOCKER 1 — The import-boundary check is a bypassable denylist

Evidence:

- `desktop/scripts/check-interactive-boundaries.mjs:31-52` rejects a few patterns and named modules.
- `desktop/scripts/check-interactive-boundaries.mjs:67-75` recognizes static import/export syntax with a regular expression.
- `desktop/scripts/check-interactive-boundaries.mjs:96-110` compares only the captured specifier and final module-name segment.
- `desktop/scripts/check-interactive-boundaries.mjs:112-117` claims the simulation adapter has the strongest boundary but adds only another `@tauri-apps/` text check.

`importLines` returns only the module specifier, but the two wrapper patterns at lines 33-34 begin with `from ...`. Those patterns can never match the stripped specifier passed at line 97. As a result, even the exact app Tauri wrapper is not rejected unless its final name happens to enter the separate denylist.

This does not enforce the contract that Lab/shared visual code cannot acquire app authority. Examples that the current rules do not reject include:

```ts
import { someCommand } from '@/lib/tauri';
import { someCommand } from '@/lib/tauri.ts';
import SomeController from '../../../components/SomeNewController.tsx';
const module = await import('@/lib/tauri');
```

The first two are ordinary forms supported by this TypeScript configuration; a new controller name or barrel can also evade the fixed module-name set. The script comment says the simulation adapter may not import anything from the app layer, but the implementation does not enforce that statement.

Why this blocks scaling:

Lab isolation must be structural, fail closed, and maintainable as new commands/components appear. A growing denylist turns every new app module into an unreviewed escape hatch and conflicts with Agora’s whitelist-first security convention.

Required correction:

1. Replace module-name denial with area-specific allowed import roots.
2. Resolve local specifiers, including aliases and explicit extensions, before deciding whether an edge is allowed.
3. Cover static imports, re-exports, dynamic imports, and any supported require-like form; using the TypeScript parser/resolver is preferred over expanding the regex.
4. Separate production and test allowances. Production `domain/`, `visual/`, and `lab/` may depend only on their documented internal layers plus narrowly allowed framework packages such as React. Test files may additionally use Vitest, Testing Library, and Node test helpers.
5. Treat unknown/unresolvable local imports as failures, not passes.
6. Add negative fixtures proving the checker rejects an alias with `.ts`, an explicit relative component extension, a barrel that resolves outside the allowed layer, and a dynamic import.

Acceptance evidence for re-review:

- current interactive production code passes the new allowlist;
- every negative fixture fails for the expected resolved edge;
- `npm run build` still runs the check before TypeScript/Vite;
- `simulationAdapter.ts` can import only scenario/domain/Lab-safe modules and cannot accept an app-layer dependency through a barrel.

### BLOCKER 2 — Visual intents can bypass the serious-decision gate

Evidence:

- `desktop/src/features/interactive/lab/LabShell.tsx:111-117` routes action-list decisions through `handleDecision`, which checks `decision.danger` and opens confirmation.
- `desktop/src/features/interactive/lab/LabShell.tsx:125-131` maps a visual intent to `LabEvent` and dispatches it directly, without resolving the corresponding `LabDecision` or checking danger/disabled/current-checkpoint state.
- `desktop/src/features/interactive/lab/scenarios/modIt.ts:207-214` declares `replace-terrain-overhaul` dangerous and supplies the required action-specific consequence copy.
- `desktop/src/features/interactive/lab/scenarios/modIt.ts:364-370` maps the graph’s `propose-remove` intent directly to that dangerous decision.
- `desktop/src/features/interactive/visual/ContentGraph.tsx:318-325` exposes the alternate `Stage removal` gesture.

Result: the decision-list button opens `Confirm replacement`, but the graph’s `Stage removal` button dispatches the same dangerous decision immediately and changes the proposal without showing that confirmation.

Why this blocks scaling:

Input parity must mean that pointer, keyboard, list, and spatial routes reach the same safety gate. A scenario-specific alternate path must not decide whether confirmation applies. This exact pattern would be unsafe if copied into live intent handling later.

Required correction:

1. Introduce one shell-owned `requestDecision(decisionId, origin)` path for action-list and visual-intent decisions.
2. Resolve the ID against `scenario.checkpoints[state.checkpoint].decisionsFor(state)` at the moment of request.
3. Reject IDs that are unavailable, disabled, or not valid for the current checkpoint.
4. If the current decision is dangerous, retain its current title/body and open confirmation; otherwise dispatch it.
5. Do not let `intentToDecision` return an already-authorized dispatch event. It may identify a proposed decision, but the shell gate owns authorization.

Acceptance evidence for re-review:

- a LabShell test reaches replacement through the graph’s `Stage removal` control and proves the proposal does not change before confirmation;
- cancelling leaves current and proposed state unchanged;
- confirming follows the same result as the action-list route;
- an out-of-checkpoint/stale decision ID is ignored with safe feedback rather than reduced.

### BLOCKER 3 — The serious confirmation is not exclusive, focus-managed, or revalidated

Evidence:

- `desktop/src/features/interactive/lab/LabShell.tsx:64` stores only a `LabDecision` as pending confirmation.
- `desktop/src/features/interactive/lab/LabShell.tsx:119-123` dispatches that stored ID on confirm without checking that it is still a current valid decision.
- `desktop/src/features/interactive/lab/LabShell.tsx:277-319` leaves the underlying decision controls mounted and active.
- `desktop/src/features/interactive/lab/LabShell.tsx:321-352` renders an inline `div role="alertdialog" aria-modal="true"`, but does not move focus, trap focus, make the background inert, handle Escape, or restore focus to the invoking control.
- `desktop/src/features/interactive/lab/LabShell.test.tsx:79-87` asserts presence/click behavior only; it does not exercise keyboard focus or background exclusion.

The DOM currently claims modality without providing modal behavior. A player can activate another decision while the confirmation is open and then confirm the stale pending decision. For example, `Cancel restore` remains actionable behind a pending restore confirmation, after which the old Confirm button can still restore.

Why this blocks scaling:

The shell is intended to own every serious Lab decision and later provide the interaction grammar that hands live work to Standard review. A non-exclusive dialog is both an accessibility failure and a stale-intent path.

Required correction:

1. Use an established accessible dialog primitive/pattern with modal background exclusion and focus containment.
2. Move focus to a safe initial target (normally Cancel for a destructive consequence), support Escape as cancel, and restore focus to the invoking control.
3. Disable or make the underlying adventure inert while confirmation is open.
4. Store enough origin/revision context to re-resolve the decision against current scenario/checkpoint state immediately before dispatch.
5. If the decision is no longer valid or its consequence copy changed, close/refuse confirmation and show safe feedback.

Acceptance evidence for re-review:

- `userEvent` keyboard tests prove initial focus, Tab containment, Escape cancel, focus return, and no background activation;
- a stale-decision test changes or invalidates the underlying state and proves old confirmation cannot dispatch;
- the action-specific replacement and restore copy retained from Terra’s fix remains correct.

### BLOCKER 4 — `ChangeStaging` exposes an operation-like callback outside `VisualIntent`

Evidence:

- `desktop/src/features/interactive/visual/ChangeStaging.tsx:17-25` accepts both `onIntent` and an opaque `onReview?: () => void`.
- `desktop/src/features/interactive/visual/ChangeStaging.tsx:107-114` calls `onReview` for the consequential review/apply control instead of emitting a declared `VisualIntent`.
- `desktop/src/features/interactive/lab/ScenarioView.tsx:129-143` uses that callback to dispatch `apply-plan` directly.
- `docs/interactive/SAFETY_BOUNDARIES.md:43-51` requires shared visuals to emit intent rather than acquire operation authority, and lines 188-190 require tests proving visuals emit only declared intents.

The component itself does not import Tauri, but the untyped callback creates an authority-shaped escape hatch that bypasses source, capability, freshness, and later live-controller gates. It also contradicts the component comment that it emits `VisualIntent`s only.

Required correction:

1. Remove the operation-like `onReview` callback from the shared visual.
2. Add one narrowly defined review intent to the closed union, such as `review-staged-changes` with stable presentation IDs only. This narrow contract extension is approved by this review.
3. Lab maps that intent to a current scenario decision through the single decision gate from BLOCKER 2.
4. A future live host must attach its scene revision and route the same intent through the live controller; it must not bind a backend call directly.
5. Keep non-authority presentation callbacks such as selection controlled, but do not add more opaque action callbacks.

Acceptance evidence for re-review:

- component tests assert the exact review intent payload;
- a contract test rejects operation-like shared-visual props/routes outside the intent union;
- the Mod It apply path remains simulated, visually current after apply, and does not call an app operation.

## 6. FIX BEFORE LIVE MODE

These do not block the remaining Lab after the four blockers pass, but must be resolved before live read adapters are considered stable or any SOL-2 action is proposed.

### FIX BEFORE LIVE MODE 1 — Source, freshness, and availability are not fail-closed throughout the visuals

- `domain/guards.ts:80-83` treats `refreshing` as executable. A refreshing view must wait for or trigger the mandatory re-read, not pass as fresh.
- `ContentGraph` and `ChangeStaging` receive both `scene` and a separate `source`; mismatched values cannot be detected by `sourceGate`. Prefer one scene/envelope authority.
- `ContentGraph.tsx:303-326` labels locked/busy nodes but still offers install/remove actions based only on global capability and current presence. It also leaves stage actions visible after an identical proposal already exists.
- `RecoveryTimeline.tsx:131-142` offers restore based on global capability without checking the selected snapshot’s `availability`.

Before live mode, make `refreshing`, `stale`, and `unknown` non-executable; eliminate duplicate source inputs; suppress/disable duplicate and unavailable actions with persistent reasons; and retain the controller’s mandatory re-read/availability gate.

### FIX BEFORE LIVE MODE 2 — Relationship rendering collapses uncertainty into false certainty

`ContentGraph.tsx:69-123` renders every non-satisfied requirement as `Missing` and every non-satisfied conflict as `Blocking`. That incorrectly converts `indeterminate` into a known failure. The linear `RelationshipRow` also emphasizes whether a target node exists rather than the authoritative relationship state.

Before live mode, render all four states explicitly in both diagram and linear views: satisfied, missing/conflicting, indeterminate, and unavailable/unknown where appropriate. Never infer state from `toId` or prose.

### FIX BEFORE LIVE MODE 3 — ContentGraph is not ready for large live instances

`ContentGraph` repeatedly scans arrays (`nodeById`, per-node relationship filters, global filters) and renders every node/relationship/action. The current shape is approximately O(nodes × relationships) and creates many ordinary Tab stops instead of the roving-focus/structured-list architecture in SOL-0.

Before live mode:

- normalize nodes with a memoized ID map and pre-group relationships once per scene revision;
- cap/progressively disclose the spatial viewport while retaining a complete searchable/linear path;
- add a bounded large-instance fixture and render/interaction budget;
- implement roving focus or an equivalent composite keyboard model so Tab reaches regions/actions rather than every graph link;
- restore focus by stable ID after refresh.

### FIX BEFORE LIVE MODE 4 — Snapshot ordering and availability need authoritative fields

`RecoveryTimeline.tsx:59` sorts `createdAt` lexicographically. The Lab supplies display strings such as `Today`, `Yesterday`, and `3 days ago`, so the declared newest-first order is not chronological. The component also ignores snapshot availability when exposing restore.

Split sortable time from display text (for example, ISO/epoch plus `createdLabel`), preserve stable tie-breaking, and gate compare/restore by availability with a reason. Live adapters must map authoritative timestamps rather than localized labels into sort keys.

## 7. SAFE DEBT

### SAFE DEBT 1 — Scenario registration and branch resume are brittle but simulation-contained

`ScenarioView.tsx:205-216` switches on string IDs and casts generic scene types. Adding the next three adventures requires another central case and can fail to a blank view if registration/view IDs drift. In addition, progress persists only a checkpoint; `initialScene(checkpoint)` reconstructs a canonical branch. For example, a player who skipped optional textures can resume at a canonical state that included them.

This is safe while Lab remains simulated and Restart is visible, but DeepSeek should either use a typed scenario/view registry or add exhaustive registration tests. Complex Fix It/Take It Offline branches should resume at a clearly documented safe checkpoint rather than silently reversing a meaningful choice.

### SAFE DEBT 2 — Feedback has two sources of truth and tests accept hidden-only success text

Every scenario returns `LabReduction.feedback` while keeping `state.lastFeedback` null. `LabShell.tsx:217` renders its visible feedback panel from `lesson.lastFeedback`, so that panel is normally dead; success/caution messages survive primarily in the screen-reader-only `Announcement`. Existing tests use `getByText`, which also finds `sr-only` content and therefore does not prove visual feedback.

Unify feedback ownership or remove the dead field. Tests that claim visible feedback should use `toBeVisible`; announcement tests should assert live-region semantics separately.

### SAFE DEBT 3 — Optional Lab code is eagerly included in the Standard bundle

`App.tsx:28` statically imports `LabShell`, which imports all scenario modules. This is acceptable for three small slices, but the remaining adventures and future visuals will increase startup cost for players who never open Lab. Route-level lazy loading should be considered before the full six-adventure set lands. Do not misattribute the entire current Vite chunk warning to Lab without a before/after measurement.

### SAFE DEBT 4 — Navigation/Guide contracts and phase status are duplicated

`domain/intents.ts` manually duplicates Guide topic IDs and a subset of the app destination union. Current scenarios are valid, but the test validates them against the same duplicated constants rather than `GUIDE_TOPICS`/`Destination`, so future drift is possible. Add a safe build-time cross-contract check without importing app authority into Lab production code.

Also update `IMPLEMENTATION_STATUS.md` after the blocker fix: its top status/phase table still says TERRA-4 is pending. The prior handoff does not list the five modified `web/public/screenshots/*.png` files visible in the worktree; DeepSeek should identify whether those are intentional documentation updates or test artifacts, without discarding them blindly.

## 8. OPTIONAL

### OPTIONAL 1 — Terra’s high-readability wordmark polish

Terra’s remaining P3 finding—sidebar wordmark truncation at 1366×768 under High readability—is outside the Lab architecture gate. It remains worthwhile app-shell polish and should be routed back to the appropriate UX/implementation pass without delaying the blocker fixes.

## 9. What passed and must be preserved

- Current `domain/`, `visual/`, and `lab/` production imports are clean; no Tauri wrapper, backend DTO, live controller, credential, path, receipt, fingerprint, or raw manifest crossed the boundary.
- `VisualScene` has an explicit simulation/live discriminator and the current scenarios consistently use namespaced `lab:<scenario>:...` IDs.
- The lesson engine is pure and has no timing-dependent completion or external service.
- Lab progress is versioned, local, and limited to non-sensitive metadata.
- Reduced motion honors the app’s `data-motion` value and OS fallback without importing the settings layer.
- Current/proposed/applied/restored state on Terra’s tested paths is materially clearer after DeepSeek’s fixes.
- Build It demonstrates isolation, Mod It shows relationship sockets, and Undo It explicitly exposes the world/save boundary and undo return point.
- Standard UI remains available, no High Interaction mode was prematurely added, and no real backend operation is wired.

Blocker fixes must not weaken these properties or expand into the remaining adventures.

## 10. Re-review checklist

Sol’s SOL-1 re-review will check only:

1. the allowlist boundary and negative bypass fixtures;
2. one decision gate shared by action-list and visual-intent routes;
3. modal/focus/stale-decision confirmation behavior;
4. the closed review intent replacing `ChangeStaging.onReview`;
5. targeted tests plus the existing 71-test baseline/build;
6. preservation of Terra’s corrected primary-path meaning.

If these pass without new authority or accessibility regressions, Sol can authorize the remaining Lab adventures. The FIX BEFORE LIVE MODE list will carry forward to the read-only adapter gate. SOL-2 remains the only place that may approve consequential live operations.

## 11. Mandatory handoff

```text
Agent: Sol
Phase: SOL-1 architecture scaling gate — first pass, BLOCKED
Commit / branch / dirty status: master tracking origin/master; uncommitted shared worktree with prior interactive changes plus this new review document
Files changed by Sol: docs/interactive/ARCHITECTURE_REVIEW_SOL.md only
Tests run: npm run check:boundaries (pass, 32 files); npm run test:unit (71/71 pass); npm run build (pass with existing >500 kB chunk warning); static source/contract review; full Playwright not repeated (DeepSeek reports 241/241 and Terra completed black-box retest)
How to launch/test: cd desktop; npm run dev; open Agora Lab. For blockers, test Mod It through the graph Stage removal route; test confirmation with keyboard focus/Escape/background exclusion/stale state; run negative import fixtures through npm run check:boundaries
Known failures: four SOL-1 blockers above; live adapters/High Interaction/remaining adventures intentionally absent; Terra P3 wordmark polish remains non-gating; implementation status header is stale; modified screenshot assets are present but were not declared in the prior handoff
Decisions made: SOL-1 does not authorize scaling yet; approved a narrow review-staged-changes VisualIntent extension; required a fail-closed import allowlist, a single Lab decision gate, and accessible revalidated modal confirmation; preserved FIX BEFORE LIVE MODE items for future gates
Decisions explicitly not made: no implementation fixes; no remaining adventure authorization; no live adapter/action approval; no offline-readiness query; no rollback/revert of prior dirty files; no SOL-2 review
Required next agent: DeepSeek V4 Flash to fix BLOCKER 1-4 and add targeted tests, then Sol for SOL-1 re-review
Why work is stopping: the SOL-1 plan requires DeepSeek fixes and re-review when BLOCKERs exist; Sol must not take over implementation
```
