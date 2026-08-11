# SOL-1 Architecture Scaling Review

Status: **APPROVED — SOL-1 passed; remaining simulated Lab adventures authorized**

Review date: 2026-08-09

Reviewer: Sol

Scope: the interactive domain and visual framework, Lab shell/lesson engine, Build It, Mod It, Undo It, DeepSeek’s Terra-fix and SOL-1-blocker-fix batches, and Terra’s TERRA-4 retest

No product code was changed during any Sol review.

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
Tests run: npm run check:boundaries (pass, 32 files); npm run test:unit (71/71 pass); npm run build (pass with existing >500 kB chunk warning); static source/contract review; full Playwright not repeated (DeepSeek reports 241/241)
How to launch/test: cd desktop; npm run dev; open Agora Lab. For blockers, test Mod It through the graph Stage removal route; test confirmation with keyboard focus/Escape/background exclusion/stale state; run negative import fixtures through npm run check:boundaries
Known failures: four SOL-1 blockers above; live adapters/High Interaction/remaining adventures intentionally absent; Terra P3 wordmark polish remains non-gating; implementation status header is stale; modified screenshot assets are present but were not declared in the prior handoff
Decisions made: SOL-1 does not authorize scaling yet; approved a narrow review-staged-changes VisualIntent extension; required a fail-closed import allowlist, a single Lab decision gate, and accessible revalidated modal confirmation; preserved FIX BEFORE LIVE MODE items for future gates
Decisions explicitly not made: no implementation fixes; no remaining adventure authorization; no live adapter/action approval; no offline-readiness query; no rollback/revert of prior dirty files; no SOL-2 review
Required next agent: DeepSeek V4 Flash to fix BLOCKER 1-4 and add targeted tests, then Sol for SOL-1 re-review
Why work is stopping: the SOL-1 plan requires DeepSeek fixes and re-review when BLOCKERs exist; Sol must not take over implementation
```

## 12. SOL-1 re-review — 2026-08-09

### 12.1 Verdict

**BLOCKED. The remaining Lab adventures are not authorized yet.**

DeepSeek closed the active visual-intent bypass and removed the concrete `ChangeStaging.onReview` escape hatch. The Radix confirmation now provides genuine modal background exclusion, safe initial focus, Escape cancellation, and invoking-control focus return. The current production slice also remains simulation-only and preserves Terra's corrected primary-path meaning.

Two residual correction batches still fail the exact first-pass acceptance contract: static boundary/visual-contract enforcement is not fully fail-closed, and confirm-time revalidation is not bound to the consequence the player reviewed. These are scaling-gate issues, not findings against the current simulated outcomes.

### 12.2 Verification performed

Current re-review baseline:

```text
npm run check:boundaries  PASS — 35 interactive files checked
fixture mode              PASS — 8/8 fixture files produced a violation
npm run test:unit         PASS — 84/84 tests in 12 files
npm run build             PASS — boundary check, TypeScript, and Vite production build
```

The production build retains the existing large-chunk warning; the main JavaScript artifact is 1,214.46 kB before gzip / 342.09 kB gzip in this worktree. Sol also ran the fixture root in ordinary mode and confirmed the expected eight individual failures for alias, explicit extension, dynamic import, re-export/barrel, relative component, Tauri-package, and disallowed-external cases. Full Playwright was not repeated; DeepSeek's handoff reports 241/241 and the re-review's unresolved findings are static contract and pure confirm-revalidation paths.

### 12.3 Original-blocker closure matrix

| Original finding | Re-review result | Evidence |
|---|---|---|
| BLOCKER 1 — bypassable import boundary | **PARTIAL — remains blocking** | The checker now resolves the required import forms and rejects all eight supplied fixtures, but still skips unclassified source areas and uses raw string-prefix containment. |
| BLOCKER 2 — visual intent bypasses serious-decision gate | **CLOSED** | Action-list and visual routes both call `LabShell.requestDecision`; `intentToDecision` returns only `{ decisionId }`; the graph Stage removal test proves no proposal mutation before confirmation and parity after confirm. |
| BLOCKER 3 — non-exclusive/stale serious confirmation | **PARTIAL — remains blocking** | Radix closes the modality/focus defects and existence/disabled-state revalidation is present, but changed consequence metadata is not compared and the required Tab-containment test is absent. |
| BLOCKER 4 — opaque `ChangeStaging.onReview` | **IMPLEMENTATION CLOSED; enforcement remains in residual batch A** | `ChangeStaging` emits the exact closed `review-staged-changes` intent and Lab routes it through the gate. The regression scanner remains syntax-bypassable. |

### 12.4 Residual BLOCKER A — Static enforcement still has fail-open cases

Evidence:

- `desktop/scripts/check-interactive-boundaries.mjs:201` silently returns for an `other` source area. A new file or folder directly under `interactive/` can therefore import an app/Tauri module without any check, even though `live/` is supposed to be the only exempt app-boundary layer.
- `desktop/scripts/check-interactive-boundaries.mjs:214` tests containment with `startsWith(interactiveNorm)` / `startsWith(rootNorm)` rather than path-segment containment. A resolved sibling such as `src/features/interactive-app/lab/controller.ts` shares the prefix, classifies as `lab`, and can be accepted as an internal Lab edge.
- `desktop/scripts/check-interactive-boundaries.mjs:165-173` and `visual/visualContract.test.ts:16` recognize callback properties only in colon/optional-property syntax. An operation-shaped method signature such as `onReview(): void` is not rejected by either guard.
- All eight negative import fixtures live inside a recognized `domain/`, `visual/`, or `lab/` area. There is no negative fixture for an unclassified source, a prefix-collision target, or callback method syntax.

Why this remains blocking:

SOL-1 is a scaling gate. A green check must continue to mean that only `live/` can acquire app authority as new adventures and shared visuals are added. The current production files are clean, but these ordinary source shapes can make the guard green while violating that invariant.

Required correction and re-review evidence:

1. Reject unclassified production source files/areas under `interactive/`; exempt only the explicitly designated `live/` boundary.
2. Determine containment with normalized path segments (for example, `path.relative`) rather than raw string prefixes.
3. Enforce operation-like shared-visual props with the TypeScript AST across property and method signatures, or an equivalently closed structural rule.
4. Add negative fixtures for all three cases and assert that each fixture fails for its intended rule.

### 12.5 Residual BLOCKER B — Confirmation is not bound to the reviewed consequence

Evidence:

- `LabShell` retains the pending `LabDecision`, but calls `revalidateDecisionForConfirm` with only `pendingConfirm.id`.
- `decisionGate.ts:47-60` re-resolves existence and `disabledReason` only. It cannot reject the same decision ID if `danger`, `confirmTitle`, or `confirmBody` changed after the dialog opened.
- `decisionGate.test.ts:54-90` covers disappearance and disabled state, but not the required same-ID/consequence-copy change.
- `LabShell.test.tsx` proves safe initial focus, Escape, background exclusion, and focus return, but does not perform the required Tab/Shift+Tab containment assertion from the first-pass acceptance list.

Why this remains blocking:

The player must confirm the consequence currently authorized, not merely a stable string ID. The present synchronous simulations make a same-ID change unlikely, but the shell is the scaling contract for later adventures and live review handoffs where state can refresh asynchronously.

Required correction and re-review evidence:

1. Bind pending confirmation to enough scenario/checkpoint/revision and consequence data to compare it with the re-resolved current decision immediately before dispatch.
2. Refuse and safely close if the decision disappears, becomes disabled/non-dangerous, or its confirmation title/body changes.
3. Add a pure same-ID changed-consequence test proving no dispatch authorization, plus `userEvent` Tab and Shift+Tab containment coverage for the dialog.

### 12.6 What passed and must remain unchanged

- No production interactive import currently reaches Tauri, an app operation, a backend DTO, a credential, or a live controller.
- Action-list and visual-intent routes share one shell-owned decision gate; visual mapping no longer returns a dispatch event.
- `ChangeStaging` emits the exact `review-staged-changes` intent, and the Mod It apply path remains simulated.
- The graph replacement route leaves current/proposed state unchanged on open and cancel, then matches the action-list outcome after confirmation.
- Radix supplies real modal background exclusion, focus containment behavior, Escape handling, and focus restoration; Cancel receives initial focus.
- Terra's corrected replacement copy, fully current post-apply Mod It state, visible post-restore Undo It state/undo point, Build It isolation, relationship sockets, and inline blocked feedback remain covered by the passing unit suite.
- The existing FIX BEFORE LIVE MODE and SAFE DEBT lists remain unchanged. No live read adapter or consequential operation is authorized by this re-review, and SOL-2 remains the only gate for live actions.

### 12.7 Mandatory re-review handoff

```text
Agent: Sol
Phase: SOL-1 architecture scaling gate — re-review, BLOCKED on two residual correction batches
Commit / branch / dirty status: master tracking origin/master at 55d7c52; shared dirty worktree containing DeepSeek's blocker-fix refinements, documentation status update, and pre-existing refreshed screenshot assets; no staging or commit performed
Files changed by Sol: docs/interactive/ARCHITECTURE_REVIEW_SOL.md only
Tests run: npm run check:boundaries (pass, 35 files); boundary fixture mode (pass, 8/8 files flagged); fixture root ordinary mode (expected failure, eight intended violations inspected); npm run test:unit (84/84 pass in 12 files); npm run build (pass with existing >500 kB chunk warning); bounded static source/contract review; full Playwright not repeated (DeepSeek reports 241/241)
How to launch/test: cd desktop; npm run dev; open Agora Lab. For the next re-review, run the new unclassified-source, prefix-collision, and callback-method negative fixtures; run the same-ID changed-consequence test; use userEvent Tab/Shift+Tab while the replacement dialog is open; then rerun check:boundaries, test:unit, and build
Known failures: boundary checker skips unclassified interactive source areas, uses prefix rather than path-segment containment, and misses method-signature operation callbacks; confirm-time revalidation does not compare danger/title/body with the reviewed decision; required Tab-containment acceptance test is missing; remaining adventures/live adapters/High Interaction/live mutation intentionally absent; existing FIX BEFORE LIVE MODE, SAFE DEBT, and Terra P3 wordmark polish remain non-gating for this fix batch
Decisions made: BLOCKER 2 is closed; BLOCKER 4's concrete onReview escape hatch is closed; BLOCKER 1 and BLOCKER 3 remain partially open; remaining Lab adventures are not authorized; prior FIX BEFORE LIVE MODE and SAFE DEBT classifications carry forward unchanged
Decisions explicitly not made: no product-code or test fixes; no remaining adventure implementation; no live adapter/action approval; no offline-readiness query; no rollback/revert of shared dirty files or screenshots; no SOL-2 review; no commit or push
Required next agent: DeepSeek V4 Flash to make the two residual correction batches fail closed and add the specified negative/interaction tests, then Sol for another bounded SOL-1 re-review
Why work is stopping: the authoritative SOL-1 plan requires Sol to report blockers and hand implementation back to DeepSeek; Sol must not take over DeepSeek's role or authorize scaling while these acceptance gaps remain
```

## 13. SOL-1 second re-review — 2026-08-09

### 13.1 Verdict

**APPROVED FOR THE REMAINING SIMULATED LAB ADVENTURES. SOL-1 IS COMPLETE.**

DeepSeek closed both residual correction batches without weakening the properties that passed earlier. The import/visual-contract guard now fails closed for the identified source and syntax variants, and confirmation dispatch is bound to the consequence the player reviewed with bidirectional keyboard-focus containment covered. All four original SOL-1 blockers are closed.

This approval authorizes DeepSeek to implement the remaining Lab adventures from the coordination plan: **Heal It, Fix It, and Take It Offline**, with Luna regression after each batch. It does not authorize live adapters out of sequence, High Interaction Mode, real mutations, or any consequential live operation. The existing FIX BEFORE LIVE MODE list remains mandatory at the live-read gate, and SOL-2 remains the only gate that may approve live actions.

### 13.2 Verification performed

```text
npm run check:boundaries  PASS — 35 interactive production files
fixture mode              PASS — all 11 negative fixture files flagged
npm run test:unit         PASS — 88/88 tests in 12 files
npm run build             PASS — boundary check, TypeScript, and Vite production build
```

Sol also ran the fixture root in ordinary mode and inspected all 11 expected failures. The three second-review fixtures fail for their intended rules: unclassified source, prefix-collision target outside the root, and operation-like method signature. The production build retains the existing large-chunk warning; the main JavaScript artifact is 1,214.79 kB before gzip / 342.21 kB gzip in this worktree. Full Playwright was not repeated; DeepSeek reports 241/241, and the second-review changes are covered by the boundary, pure gate, and focus tests above.

### 13.3 Residual-batch closure

#### Residual BLOCKER A — CLOSED

- Source classification is rooted and path-segment based; any unclassified source under the scan root is now an error, while only `live/` is exempt as the designated boundary.
- Target containment uses `path.relative` plus absolute/parent rejection, closing the `interactive-app` prefix collision.
- The visual callback guard uses the TypeScript AST and checks property and method signatures.
- Isolated negative fixtures prove the unclassified-source, prefix-collision, and callback-method cases, in addition to the original eight bypass cases.
- The current production visual props remain limited to the closed intent plus controlled presentation callbacks; no opaque operation route was introduced.

#### Residual BLOCKER B — CLOSED

- `DecisionConsequence` binds confirmation to the reviewed danger state, title, and body.
- Confirm-time revalidation re-resolves the current checkpoint decision, rejects missing/disabled decisions, and rejects any consequence mismatch before dispatch.
- Pure tests cover unchanged, missing, disabled, changed-title, changed-body, and changed-danger cases.
- `userEvent` coverage now proves Tab and Shift+Tab remain inside the modal, alongside the existing safe initial focus, Escape, background exclusion, and invoking-control focus-return checks.

### 13.4 Preserved architecture and carry-forward gates

- Lab remains deterministic, namespaced, and simulation-only; no Tauri, backend DTO, credential, live-controller, or real-instance authority crossed into `domain/`, `visual/`, or `lab/`.
- Action-list and visual-intent routes still share the shell-owned decision gate.
- `ChangeStaging` still emits the closed `review-staged-changes` intent rather than an operation callback.
- Terra's corrected primary-path meaning remains covered: action-specific replacement copy, fully current post-apply Mod It state, visible post-restore Undo It state/undo point, Build It isolation, relationship sockets, and inline blocked feedback.
- All existing FIX BEFORE LIVE MODE, SAFE DEBT, and OPTIONAL classifications in this report carry forward unchanged. SOL-1 approval does not downgrade or waive them.

### 13.5 Mandatory second re-review handoff

```text
Agent: Sol
Phase: SOL-1 architecture scaling gate — second re-review, APPROVED and complete
Commit / branch / dirty status: master tracking origin/master at 55d7c52; shared dirty worktree containing DeepSeek's SOL-1 fix batches, documentation updates, Sol's review history, and pre-existing refreshed screenshot assets; no staging or commit performed
Files changed by Sol: docs/interactive/ARCHITECTURE_REVIEW_SOL.md only
Tests run: npm run check:boundaries (pass, 35 files); boundary fixture mode (pass, 11/11 files flagged); fixture root ordinary mode (expected failure, all 11 intended violations inspected); npm run test:unit (88/88 pass in 12 files); npm run build (pass with existing >500 kB chunk warning); bounded static source/contract review; full Playwright not repeated (DeepSeek reports 241/241)
How to launch/test: cd desktop; npm run dev; open Agora Lab. Existing vertical slice: exercise Build It, Mod It, and Undo It; for the closed gates, run check:boundaries plus fixture mode, use Mod It Stage removal to verify confirmation parity, and Tab/Shift+Tab through Confirm replacement
Known failures: no remaining SOL-1 blocker; existing Vite large-chunk warning; full Playwright not independently repeated by Sol; Heal It, Fix It, and Take It Offline are intentionally not implemented yet; live adapters, High Interaction Mode, and live mutation remain unauthorized; prior FIX BEFORE LIVE MODE, SAFE DEBT, and Terra P3 wordmark items remain open at their recorded classifications; IMPLEMENTATION_STATUS.md remains DeepSeek's pre-review handoff snapshot and should be advanced when the next implementation batch starts
Decisions made: all four original SOL-1 blockers and both residual batches are closed; SOL-1 is approved; DeepSeek is authorized to implement Heal It, Fix It, and Take It Offline as simulated Lab adventures; Luna regression remains required after each batch; existing live-mode and SOL-2 gates remain mandatory
Decisions explicitly not made: no product-code/test implementation by Sol; no live adapter approval out of sequence; no High Interaction Mode; no real mutation or consequential live operation; no offline-readiness backend query; no waiver of FIX BEFORE LIVE MODE or SAFE DEBT; no commit or push
Required next agent: DeepSeek V4 Flash for the remaining simulated Lab adventures, with Luna regression after each implementation batch per the coordination plan
Why work is stopping: SOL-1 is genuinely complete and the authoritative coordination sequence now hands implementation back to DeepSeek; Sol must not write the remaining adventures or take over another agent's role
```

## 14. SOL-2 entry-gate check — 2026-08-09

### 14.1 Verdict

**NOT READY — SOL-2 has not started. No live interaction is approved or rejected.**

The SOL-2 plan says to resume only when DeepSeek has stable Lab components **plus live read-only adapters**. The first prerequisite is satisfied: all six simulated adventures exist, DeepSeek fixed Luna's L-001 narrow-layout issue, and Luna's retest records no blocking Lab regression. The second prerequisite is absent.

Evidence:

- There is no `desktop/src/features/interactive/live/` directory or other live interactive adapter/controller implementation in the source tree. The only interactive adapter files are `lab/simulationAdapter.ts` and its test.
- `IMPLEMENTATION_STATUS.md` explicitly states that no live adapters were built and identifies them as the future DEEPSEEK-6 batch.
- The remaining-adventures and L-001 handoffs explicitly list live read adapters, High Interaction Mode, and live mutation as decisions not made.
- Current source changes add simulated scenarios/visuals and responsive shell behavior only; they do not project authoritative live DTOs into `VisualScene` or expose a High Interaction read surface.

Without read-only projections, freshness/revision behavior, unavailable/indeterminate mapping, and live loading/error states, Sol cannot evaluate the proposed real gestures against the eight mandatory SOL-2 questions. Reviewing hypothetical wiring now would bypass the hard-stop sequence and produce approvals that are not tied to inspectable code.

### 14.2 Authoritative sequence correction

The current implementation-status and Luna handoffs route next to Terra TERRA-5 and then SOL-3. That conflicts with `00-COORDINATION.md`, whose sequence after the remaining-Lab/Luna batch is:

```text
DEEPSEEK live read-only adapters
    ↓ HARD STOP
SOL-2 live-operation safety gate
```

The coordination document is authoritative for handoff order. The next phase is therefore DeepSeek's live read-only adapter batch, followed directly by this SOL-2 gate. Terra's deep **live** UX review remains later, after SOL-2-approved real High Interaction actions and Luna smoke/regression; it cannot substitute for the missing adapter prerequisite.

### 14.3 Requirements for the next SOL-2 entry

DeepSeek must return with an inspectable, read-only live slice that:

1. adds the designated `interactive/live/` adapter/controller boundary without adding mutating authority to shared visuals;
2. maps minimal authoritative presentation data rather than copying giant backend DTOs;
3. exposes source identity, revision/freshness, availability, loading, empty, error, and unknown/indeterminate states fail closed;
4. addresses the existing FIX BEFORE LIVE MODE findings for freshness, source consistency, relationship uncertainty, large-instance graph behavior, and authoritative snapshot ordering/availability;
5. keeps every live gesture intent-only, with no install, disable/remove, repair, restore, crash action, or memory mutation wired before SOL-2 approval;
6. includes tests for DTO redaction, stale/refreshing behavior, unavailable states, adapter failure, boundary enforcement, and Standard-mode coexistence;
7. records for each proposed future interaction the intended existing Agora operation and review/recovery seam, without calling it yet.

Once that batch exists, Sol can inspect each of the six proposed integration classes—dependency-aware disable/remove, install planning, health/loader repair, snapshot restore, crash actions, and memory changes—and explicitly approve or reject them against the eight required questions.

### 14.4 Mandatory entry-gate handoff

```text
Agent: Sol
Phase: SOL-2 live-operation safety gate — entry check only, NOT READY / NOT STARTED
Commit / branch / dirty status: master tracking origin/master at 55d7c52; shared dirty worktree containing the completed simulated Lab/Luna batch and prior review history; no staging or commit performed
Files changed by Sol: docs/interactive/ARCHITECTURE_REVIEW_SOL.md only
Tests run: no build or automated suite run because the required live adapter implementation is absent; read-only evidence consisted of the authoritative coordination/Sol plans, implementation and Luna handoffs, full interactive source inventory, repo-wide live-adapter search, and focused App/simulation wiring diff
How to launch/test: current simulated Lab remains available with cd desktop; npm run dev. SOL-2 cannot yet be exercised. After the read-only batch, launch Standard and High Interaction side by side and test live loading/error/stale/unavailable projections before testing any intent proposal
Known failures: no interactive live adapter/controller or High Interaction read surface exists; no authoritative DTO-to-VisualScene projection exists; the SOL-1 FIX BEFORE LIVE MODE items remain open; IMPLEMENTATION_STATUS.md and Luna's handoff currently point to Terra/SOL-3 out of the authoritative sequence; no destructive live wiring is permitted
Decisions made: stable-Lab prerequisite is satisfied; live-read-only-adapter prerequisite is not; SOL-2 is not started; no proposed real interaction is approved or rejected; authoritative next phase is DeepSeek live read-only adapters, then return to Sol
Decisions explicitly not made: no product-code or test implementation; no live adapter design approval in the abstract; no dependency/install/repair/restore/crash/memory action approval; no High Interaction Mode or live mutation; no SOL-3 educational review; no Terra UX judgment; no commit or push
Required next agent: DeepSeek V4 Flash to implement the live read-only adapter layer and close the FIX BEFORE LIVE MODE requirements, with zero real mutation, then Sol for SOL-2
Why work is stopping: the explicit SOL-2 resume condition is unmet and the coordination document places a hard stop after DeepSeek's read-only adapter batch; Sol must not review or authorize nonexistent live wiring
```

## 15. SOL-2 live-operation safety gate - 2026-08-09

### 15.1 Verdict

**BLOCKED WITH PARTIAL OPERATION-SEAM APPROVAL. SOL-2 was performed, but the current live host is not authorized to enable any consequential capability.**

The entry condition is now satisfied in the narrow sequencing sense: all six Lab adventures exist and `desktop/src/features/interactive/live/` contains a production read surface. The current live imports are read-only by source inspection; Sol found no mutation command invoked by the adapter or host.

The read surface is not yet the fail-closed foundation described by SOL-0 or claimed in the DEEPSEEK-6 handoff. Independent read failures are converted into apparently fresh, validated, available state; the live host does not invoke the source/capability/freshness gate; refreshes can complete out of order; and most intents do not open a contextual Standard review at all. Passing tests therefore prove the supplied fixtures, not the end-to-end safety contract.

SOL-2 makes two decisions separately:

1. **Operation-seam decision:** whether a particular visual intent may eventually route into a named existing Standard operation under the contracts below.
2. **Current wiring authorization:** whether DeepSeek may enable that capability in the current host now.

Current wiring authorization is **NO for every consequential capability** until the cross-cutting blockers in section 15.3 are fixed and re-reviewed. The operation-seam decisions are explicit:

| Proposed integration | SOL-2 decision | Approved/rejected scope |
|---|---|---|
| Dependency-aware removal | **APPROVE the `InstallFlow` seam only** | A remove intent may open a freshly resolved canonical removal plan. It may not call a remove command or `DependencyPrompt` directly. |
| Dependency-aware disable | **REJECT** | The current plan-plus-multiple-renames path is staleable, partially committing, non-transactional, and has no recovery point. |
| Install/update planning | **APPROVE the `InstallFlow` seam only** | A gesture may create a minimal intent and open the existing resolver/review flow. No visual plan or direct apply is authoritative. |
| Health review | **APPROVE inspection/review only; REJECT direct repair** | A fresh scan may open the app-level `HealthDialog`. A visual may not invoke `disableModForTest`, auto-repair, or launch directly. |
| Loader review/change | **APPROVE the existing plan/chooser/change seam** | Re-plan on entry, retain proven-versus-indeterminate review, core lock/process rejection, metadata rollback, and fresh post-change health. |
| Snapshot compare | **APPROVE read-only compare only** | Re-read the selected snapshot, run the existing diff query, and show scope/world uncertainty. This approval does not include restore. |
| Snapshot restore | **REJECT** | The current Standard button has no serious confirmation, and the backend command does not reserve the instance against all competing operations across its check-and-restore window. |
| Crash actions | **APPROVE `open-crash-doctor` navigation only** | Re-read evidence inside `CrashInvestigator`; no visual hypothesis or experiment may directly mutate or launch. |
| Memory changes | **REJECT** | There is no dedicated current-to-proposed review bridge, stale-write protection, or authoritative applied-value response; the current visual also leaks a disabled proposal gesture. |

These approvals do not waive the common controller gate. DeepSeek may implement approved bridges behind disabled capabilities for re-review. Rejected mutation seams must remain absent.

### 15.2 Verification performed

Current SOL-2 baseline:

```text
npm run check:boundaries  PASS - 57 interactive files
fixture mode              PASS - all 11 negative fixture files flagged
npm run test:unit         PASS - 137/137 tests in 19 files
npm run build             PASS - boundary check, TypeScript, and Vite production build
```

The build retains the known chunk warning; the main JavaScript artifact is 1,268.35 kB before gzip / 355.52 kB gzip in this worktree. Sol did not repeat the full Playwright suite; DeepSeek reports 241/241, while the findings below concern default-command failure semantics, request ordering, unused gates, unavailable operation bridges, and backend concurrency paths not covered by those browser tests.

Static review traced:

- every live adapter/host import and every emitted live intent;
- `InstallFlow` resolution, fingerprint validation, cancellation, snapshot, apply, rollback, health outcome, and refresh seams;
- dependency-plan and direct enable/disable handlers;
- app-level process/health controllers and loader plan/change behavior;
- snapshot diff/restore UI plus backend undo/transaction logic;
- Crash Doctor evidence, lazy recovery snapshot, dependent disable experiment, correlated outcome, and restoration behavior;
- memory recommendation, Standard save UI, command validation, and database update behavior;
- the authoritative SOL-0 safety/architecture documents and `MASTER_SPEC.md` section 19, especially 19.15 and 19.19.

### 15.3 Cross-cutting BLOCKER findings

#### BLOCKER 1 - Partial read failures become false fresh knowledge

Evidence:

- `live/liveScene.ts:48-62` catches every command error and substitutes `null` or `[]` without retaining which read failed.
- `live/liveScene.ts:71` labels the aggregate source `fresh` whenever instance detail exists, regardless of missing process, health, snapshot, crash, memory, or Java reads.
- `live/liveScene.ts:82` maps a failed health read to zero findings.
- `LiveSceneView.tsx:76` always passes `validated` to `HealthLens`, whose zero-finding copy at `HealthLens.tsx:123-126` says the simulated instance is ready to launch even on a live health failure.
- `readAdapters/index.ts:98` calls content healthy when no failed health read remains to distinguish unknown; `readAdapters/index.ts:204` and `:272` mark snapshots and runtime available unconditionally.

Consequences include a health-command failure appearing as a clean validation, a process-state failure appearing idle/editable, a snapshot failure appearing as no snapshots, a Java/recommendation failure appearing as an available runtime, and a total Tauri failure appearing as an empty instance rather than the handoff's claimed fail-closed error state.

Required correction:

1. Retain an explicit result/availability value for every read fragment; do not erase errors into ordinary empty values.
2. Treat instance-detail failure as an adapter error or explicitly unavailable scene, not a valid empty instance.
3. Treat process-state uncertainty as non-executable and consume the canonical app-level process state where available.
4. Render failed health as not validated/unknown; never show ready or healthy from absence of a report.
5. Distinguish a successfully read empty list from an unavailable snapshot/crash/runtime fragment.
6. Derive aggregate freshness from the relevant fragments instead of overwriting every assembled scene to `fresh`.
7. Add default-loader tests for each individual read failure and total Tauri failure; injected `load()` rejection alone is not sufficient.

#### BLOCKER 2 - The live intent controller and its eight gates do not exist

Evidence:

- `domain/guards.ts` defines `gateLiveIntent`, but production search finds it only in that module; no host/controller calls it.
- `LiveInteractiveHost.tsx:108-118` special-cases selection and otherwise forwards every intent directly to `onOpenStandardOperation` without source, capability, freshness, entity availability, duplicate-review, or refreshed-state checks.
- `InstanceEditor.tsx:1115-1120` opens Crash Doctor for one intent and merely exits High Interaction for every other intent; no selected content, snapshot, health, loader, or memory context reaches an existing review surface.
- No `live/intentController.ts` or `live/operationBridges/` implementation exists, although those are the SOL-0 ownership points for availability, review, backend outcome, and refresh gates.

Required correction:

One controller must own this sequence for every approved intent:

```text
source -> capability -> freshness -> relevant re-read/entity resolution
       -> availability/duplicate-operation gate -> existing Standard review
       -> backend result or rejection -> outcome classification -> mandatory re-read
```

The controller must keep private DTOs/tokens outside presentation models, coalesce duplicate requests, preserve accessible focus/status during review, never auto-retry a stale mutation, and keep proposals non-successful after rejection, cancellation, failure, or rollback. Tests must prove that no mutation wrapper is invoked before the existing review confirms and that every terminal outcome attempts a refresh.

#### BLOCKER 3 - Capability flags are advisory rather than enforced

Evidence:

- `HealthLens.tsx:78-83` renders any finding's review intent without consulting `canReviewHealth` or `canReviewLoader`.
- `RuntimeWorkbench.tsx:109-118` emits `propose-memory` whenever current mode is manual, even when `canProposeMemory` is false.
- `RecoveryTimeline.tsx:129-146` gates compare by snapshot availability but never by `canPreviewSnapshot`.
- `CrashEvidenceBoard` declares `onIntent` and `capabilities` props but destructures neither; it cannot emit the advertised `open-crash-doctor` intent.
- `liveCapabilities.ts` therefore does not describe the controls that are actually reachable.

Required correction:

Shared visuals must hide or disable every operation-shaped command from the matching capability and retain a visible unavailable reason where appropriate. The live controller must enforce the same capability independently, so a component bug or synthetic dispatch cannot cross the gate. Add component and host tests for every capability in both states, including manual-to-automatic memory, snapshot compare, finding review, and Crash Doctor navigation.

#### BLOCKER 4 - Refresh and canonical operation state can go backwards

Evidence:

- `LiveInteractiveHost.tsx:72` replaces an existing scene with `loading` during refresh, then `:81` stores `refreshing: true` only after the read completes. The UI therefore hides the scene during the actual refresh and can show `Refreshing...` indefinitely after a source was marked fresh.
- There is no request sequence or abort rule. Two refreshes can resolve out of order and an older result can replace a newer result while being relabeled with its captured revision.
- `revisionRef` is not reset when `instanceId` changes.
- `liveScene.ts` calls `queryLaunchState` rather than consuming the app-level `useProcessController` state already passed into `InstanceEditor`. That loses launching, stopping, delegated, and app-owned pending state and contradicts the SOL-0 rule that the canonical process controller wins.
- The live scene also does not consume the app-level pack-install task, so visual availability cannot represent an active canonical install.

Required correction:

Keep the last scene visible but non-executable while refreshing, mark its source `refreshing`, accept only the latest request for the current instance, create a new revision per accepted observation, reset selection/revision safely across instance changes, and derive process/install availability from the canonical app controllers. Tests must deterministically resolve requests out of order and prove latest-wins behavior, correct refresh labeling, and no stale-intent forwarding.

#### BLOCKER 5 - The live authority boundary is not fail closed for the next phase

Evidence:

- `check-interactive-boundaries.mjs:228-231` exempts all of `live/` from import checks.
- `live/boundary.test.ts` permits any `@/lib/tauri...` import in any production live file; it does not distinguish read adapters from operation bridges or whitelist read-only commands.
- The current source happens to import only read commands, but a later mutation import in `readAdapters/` or `liveScene.ts` would keep the boundary green.

Required correction:

Classify live subareas. Read adapters/loaders may import only an explicit read-command allowlist. The intent controller may call typed operation bridges, not mutation commands. Only narrowly named operation bridges may host the already approved Standard controllers/components. Unknown live files and direct mutation imports outside those bridges must fail the build, with negative fixtures for representative install, disable, restore, launch, and settings imports.

### 15.4 Dependency-aware disable and remove

1. **Gesture:** removal is `ContentGraph`'s Stage removal -> `propose-remove(contentId)`. Disable requires an accessible `propose-enabled(contentId, false)` command; no current live visual emits it.
2. **Existing operation:** removal must become `InstallIntent { action: remove | batch-remove }` and open `InstallFlow`. Disable currently uses `getDisablePlan` -> `DependencyPrompt` -> one or more `disableInstanceMod` calls in `InstanceEditor.tsx:581-637`.
3. **Preview/review:** `InstallFlow` supplies the authoritative reverse-dependency/file/snapshot review for removal. `DependencyPrompt` previews dependents for disable, but required candidates can be unchecked and the backend never receives an authority-bearing plan.
4. **Recovery:** canonical removal has the mandatory recovery snapshot, reversible apply, health rollback, and explicit outcome. Ordinary disable has no snapshot, transaction, undo point, or multi-file rollback.
5. **Stale data:** removal is protected by registry revision, plan fingerprint, and live instance-state hash in `install_pipeline.rs:1318-1390`. Disable plans have no token/revision; files/dependencies can change between planning and the sequence of renames.
6. **Backend rejection:** removal stays in the canonical failed/rolled-back outcome. Disable can reject after earlier filenames were already changed, leaving a partial selected set.
7. **Locks/cancellation:** removal retains install cancellation and one-active-install behavior, but execution still needs the process/launch exclusion required in section 15.5. Disable holds a core instance lock per rename only; the group is neither atomic nor cancellable.
8. **False success:** removal may animate only after a successful outcome and re-read. Disable cannot animate a group success from individual calls; the current path has no authoritative group result.

**Decision:** **APPROVE removal through `InstallFlow` only. REJECT disable.** A future disable re-review requires a backend-owned, dependency-aware atomic operation (or canonical pipeline action) that re-resolves under the instance lock, rejects incomplete required-dependent choices, checks process/snapshot readiness, creates recovery, rolls back the complete set on failure, and returns one explicit outcome.

### 15.5 Install and update planning

1. **Gesture:** a supported not-installed/update candidate may emit `propose-install` or `propose-update`, followed by a staged review command. The current live adapter exposes installed mods only and `ContentGraph` has no update control, so DeepSeek must not invent an install target by parsing a visual ID.
2. **Existing operation:** construct only a minimal `InstallIntent`, then open `InstallFlow`; `resolveInstallPlan`, `applyInstallPlan`, and `cancelInstall` remain the sole path.
3. **Preview/review:** the existing flow owns dependencies, conflicts, optional choices, file additions/removals/disables, loader choice, snapshot estimate, and final review. The visual preview is non-authoritative.
4. **Recovery:** mandatory pre-apply snapshot, reversible application, post-apply health, rollback, and explicit cancellation/failure outcomes remain unchanged.
5. **Stale data:** the controller re-reads the target and resolves a new plan; core rechecks fingerprint, registry revision, and instance-state hash at apply. A stale rejection returns to review; it is never auto-applied or automatically retried.
6. **Backend rejection:** preserve the proposal and show the existing resolver/executor error. Refresh before presenting current state; do not paint installed/updated.
7. **Locks/cancellation:** keep the app-level active install state and acknowledged cancellation. Before execution is enabled from High Interaction, the authoritative path must also demonstrate rejection of a running/launching instance and competing instance mutation; `apply_install_plan` currently registers an active install but does not itself inspect `running_process`/`launch_reservation`.
8. **False success:** only the canonical success outcome followed by a successful live re-read may commit the visual state. Progress or staging animation is never success.

**Decision:** **APPROVE the `InstallIntent` -> `InstallFlow` bridge design for install/update.** In the next batch it must remain behind disabled capabilities until the common controller and process-exclusion acceptance evidence pass. No direct `applyInstallPlan` call is permitted from a visual or generic live host.

### 15.6 Health and loader review/repair

1. **Gesture:** an explicit Review health command emits `review-health`; a structured loader finding/candidate emits `review-loader(candidateId?)`. The current live `HealthLens` emits only loader review from selected findings and has no general health-review command.
2. **Existing operation:** the controller runs a fresh `checkInstanceHealth` and opens the app-level `HealthDialog`; loader review runs `planLoaderChange` and opens the existing `LoaderChooser`/health loader card. Loader commit stays `changeLoaderVersion` or the app-level switch-and-retry controller.
3. **Preview/review:** `HealthDialog` retains blocker/warning/recommendation semantics and scan identity; `LoaderChooser` retains proven versus indeterminate evidence and affected-mod review.
4. **Recovery:** loader change installs before metadata commit, rolls metadata back on persistence conflict/failure, invalidates caches, and returns fresh health. A general health repair that disables content currently calls `disableModForTest` directly at `HealthDialog.tsx:333-337`; it has no dependency review or recovery snapshot and is not approved.
5. **Stale data:** never pass the presentation finding as authority. Re-scan/re-plan on bridge entry; launch continues using the private health scan token, while loader core rechecks the instance tuple and signed catalog under lock.
6. **Backend rejection:** leave the dialog open with the error, refresh health/loader evidence when possible, and do not clear or mark the finding repaired.
7. **Locks/cancellation:** loader core acquires the instance lock and rejects an active core-managed process. Do not advertise cancellation for a non-cancellable loader commit. Direct health-disable repair remains rejected until it has dependency/process/recovery authority.
8. **False success:** loader movement or a validation sweep may illustrate only a returned change plus refreshed health. A clean animation cannot manufacture a clean health result.

**Decision:** **APPROVE fresh health inspection/review and the existing loader plan/chooser/change bridge. REJECT direct visual health repair and the current `HealthDialog` disable seam as a High Interaction action.** The live adapter's false-ready behavior must be fixed before either approved review bridge is exposed.

### 15.7 Snapshot compare and restore

1. **Gesture:** select a return point -> Compare emits `preview-snapshot`; a separate serious Restore command would emit `request-snapshot-restore` only from the compare/review surface.
2. **Existing operation:** compare re-resolves the snapshot then calls `detectDrift`. Restore would call the existing `restore_snapshot` backend command only after review.
3. **Preview/review:** the current Standard page has Show diff, but its Restore button at `InstanceEditor.tsx:1704` calls the backend immediately. There is no serious confirmation that binds snapshot identity, changed files, scope, and world/save uncertainty to the reviewed consequence.
4. **Recovery:** the backend correctly creates a full pre-restore undo snapshot and the core restore verifies staged bytes, swaps tracked roots, and rolls back partial promotion failures.
5. **Stale data:** a selected snapshot must be re-listed and re-diffed immediately before confirmation; any changed/missing snapshot, process state, scope, or diff invalidates the old review. The current live adapter marks every listed snapshot available.
6. **Backend rejection:** retain the selected point and report busy/missing/verification/restore/rollback outcomes distinctly, then refresh. Never convert rejection to an empty timeline or success.
7. **Locks/cancellation:** the command checks direct running/launch-reservation state before spawning restore, but it does not reserve the instance across the following undo-snapshot/restore work or exclude an active install through the same authority. Restore has no supported cancellation and must not advertise one.
8. **False success:** the timeline remains current/proposed until restore returns and the re-read proves the selected state. A rewind animation cannot signal completion.

**Decision:** **APPROVE read-only compare after the common controller fixes. REJECT snapshot restore.** Re-entry requires a real serious confirmation, fresh identity/diff binding, authoritative operation reservation against launch/install/other mutations, explicit rollback-failure reporting, and controller tests for stale, busy, failure, rollback, and refresh outcomes.

### 15.8 Crash actions

1. **Gesture:** a capability-gated Open Crash Doctor button emits `open-crash-doctor`. `CrashEvidenceBoard` currently renders no such control, so the `InstanceEditor` callback is unreachable.
2. **Existing operation:** open `CrashInvestigator` for the instance and let it collect fresh evidence. No visual hypothesis ID, score, filename, or experiment state authorizes an action.
3. **Preview/review:** Crash Doctor remains read-only first, shows hypotheses as hypotheses, obtains dependency review for a disable experiment, and uses the canonical launch controller.
4. **Recovery:** `CrashInvestigator` lazily creates a recovery snapshot before its first mutation and restores on failed-to-start, crash/inconclusive, abandonment, or close without confirmed success.
5. **Stale data:** opening the Doctor discards visual authority and re-investigates. A changed crash starts a new hypothesis and never confirms the old one from one outcome.
6. **Backend rejection:** if mutation occurred, attempt restoration and report mutation and restoration failures separately. If launch is rejected, do not treat it as experimental evidence.
7. **Locks/cancellation:** the Doctor must disable experiment controls while the instance is launching/running/stopping or another operation is active. Today it creates the snapshot and disables files before `onLaunch` can reject an already active launch, so that pre-mutation availability check needs correction even though only navigation is approved here.
8. **False success:** retain the current correlated-outcome and player-confirmation semantics. The board must never show `Recovery point ready` before a snapshot exists; `readAdapters/index.ts:242` currently does exactly that.

**Decision:** **APPROVE `open-crash-doctor` as navigation to the existing Doctor only. REJECT any direct visual experiment/disable/launch action.** Add the missing button, gate it in both component and controller, correct recovery readiness, and enforce pre-mutation process/operation availability inside the Doctor before exposing the navigation bridge.

### 15.9 Memory changes

1. **Gesture:** Stage manual choice or Use recommended emits `propose-memory(mode, memoryMiB?)`. The manual-to-automatic control currently emits despite `canProposeMemory: false`.
2. **Existing operation:** the only mutation seam is `updateInstanceJvm` in the broad Standard Java settings section. There is no focused bridge that re-reads current settings, stages only the requested memory choice, and asks for a current-to-proposed summary.
3. **Preview/review:** a dedicated Standard review must show current mode/value, recommendation/headroom, proposed effective value, whether it applies on the next launch, and unchanged JVM/GC/Java fields. Merely exiting High Interaction is not review.
4. **Recovery:** cancel leaves the database unchanged; backend failure preserves the prior row; after success, preserve the previous setting as an explicit revert choice or make re-entry straightforward. A snapshot is not required for a database-only setting, but silent partial field changes are forbidden.
5. **Stale data:** re-read the current row/recommendation immediately before review and again at commit through an expected-current revision/value or equivalent compare-and-set. Do not let an old recommendation overwrite a newer manual choice.
6. **Backend rejection:** show validation/conflict failure and keep the proposal uncommitted. The current backend silently clamps memory to 2048-32768 and normalizes unknown modes, then returns `void`, so the caller cannot distinguish its proposal from the effective saved value without re-reading.
7. **Locks/cancellation:** serialize concurrent setting saves. If changing memory while a game is running is allowed because it affects only the next launch, say so explicitly and never imply the current process changed; otherwise reject via canonical process state. There is no long-running cancellation contract.
8. **False success:** `RuntimeWorkbench` currently calls the configured heap value "in use," which is not runtime usage. Show configured/effective-next-launch state, and mark committed only after the backend response plus re-read.

**Decision:** **REJECT memory changes.** Re-entry requires a focused review bridge, capability enforcement, truthful configured-versus-running copy, strict/effective backend validation response, stale-write protection, and success/failure/revert tests.

### 15.10 Other required read-surface corrections

These do not change the operation decisions above but are required for the SOL-2 re-review:

- `presentationPreference.ts` is tested but unused; `InstanceEditor.tsx:1097` always starts a local `useState(false)`. Either wire the versioned preference according to SOL-0 or stop claiming it is implemented.
- `readAdapters/index.ts:115` and `:133-134` retain O(nodes x relationships) scans, so FIX BEFORE LIVE MODE 3 is not fully closed at adapter scale even though the component now caps and indexes its render. Normalize target lookup and relationship summaries once.
- Relationship data currently comes only from loader-compatibility health findings, not a dependency-authoritative read. Do not enable dependency gestures or imply a complete dependency graph from that projection.
- The host must preserve Standard view as the fallback for any unsupported/unavailable fragment and must not label the whole surface "Read-only - actions open the Standard review first" until each reachable action actually opens a contextual review.

### 15.11 Re-review acceptance checklist

DeepSeek should return to Sol with:

1. all five cross-cutting blockers fixed and targeted tests passing;
2. truthful per-fragment availability and failure rendering using the default command loader;
3. a latest-wins refresh/controller with canonical process and install state;
4. fail-closed capability enforcement in visuals and controller;
5. structural live subarea/import enforcement;
6. contextual bridges for approved health review, loader review/change, snapshot compare, Crash Doctor navigation, and InstallFlow planning, kept behind disabled consequential capabilities until Sol verifies them;
7. no direct disable, health repair, snapshot restore, crash experiment, or memory mutation bridge;
8. proposed backend/controller designs and tests for the rejected seams if DeepSeek wants them reconsidered;
9. corrected DEEPSEEK-6 handoff claims, especially partial-read failure, browser failure, preference, and FIX BEFORE LIVE MODE closure;
10. the existing boundary/unit/build gates plus focused operation-controller and concurrency tests. Full Playwright should be repeated once an approved bridge is actually enabled, not merely for a disabled scaffold.

### 15.12 Mandatory SOL-2 handoff

```text
Agent: Sol
Phase: SOL-2 live-operation safety gate - BLOCKED with partial operation-seam approval
Commit / branch / dirty status: master tracking origin/master at 55d7c52; shared dirty worktree containing DeepSeek's Lab/live-adapter batches, prior Sol/Terra/Luna documentation, and refreshed documentation screenshots; no staging, commit, or push performed
Files changed by Sol: docs/interactive/ARCHITECTURE_REVIEW_SOL.md only
Tests run: npm run check:boundaries (pass, 57 files); boundary fixture mode (pass, 11/11 files flagged); npm run test:unit (137/137 pass in 21 files); npm run build (pass with existing >500 kB chunk warning; main JS 1,274.26 kB / 357.25 kB gzip); git diff --check (pass with existing LF/CRLF conversion warnings); static end-to-end seam review; full Playwright not repeated (DeepSeek reports 241/241)
How to launch/test: cd desktop; npm run dev; open My Instances -> an instance -> High Interaction view. Exercise total/partial read failure in Tauri and rapid overlapping Refresh requests; verify Standard escape. For operation re-review, run the controller tests in section 15.11 before enabling any capability
Known failures: partial reads become false fresh/validated/available state; gateLiveIntent is unused; capability flags are bypassed or unreachable in several visuals; refreshes are not latest-wins and refreshing state is reversed; canonical process/install state is not consumed; all live/ imports are structurally exempt; versioned presentation preference is unused; adapter graph work remains superlinear/incomplete; current Standard disable, health-disable, snapshot-restore, Crash Doctor preflight, and memory seams have the operation-specific gaps recorded above
Decisions made: SOL-2 entry is satisfied and the gate was performed; removal/install/update may use InstallFlow only; fresh health inspection, loader review/change, read-only snapshot compare, and Crash Doctor navigation retain their narrow approvals; dependency disable, direct health repair, snapshot restore, direct crash experiments, and memory changes remain rejected; no consequential capability may be enabled before common-blocker re-review
Decisions explicitly not made: no product-code/test fixes; no direct mutation implementation; no waiver of backend locks, recovery, stale-state, cancellation, or outcome refresh; no offline-readiness live query; no Terra UX redesign; no SOL-3 educational review; no git staging/commit/push; no modification or rollback of another agent's dirty files
Required next agent: DeepSeek V4 Flash to fix the cross-cutting live-read/controller blockers and implement only the approved bridges behind disabled capabilities, then Sol for a bounded SOL-2 re-review before any consequential capability is enabled
Why work is stopping: SOL-2 found architecture and safety blockers and explicitly rejects several current operation seams; the coordination plan hands implementation back to DeepSeek, while Sol must not take over product code or permit destructive live wiring without a passing re-review
```

## 16. SOL-2 bounded re-review - 2026-08-09

### 16.1 Verdict

**BLOCKED. DEEPSEEK-7 closes useful parts of the read-surface foundation, but it does not satisfy the section 15.11 acceptance checklist and does not receive current-wiring authorization for any consequential live capability.**

The operation-seam decisions in section 15.1 are unchanged. Install/update/removal may eventually enter `InstallFlow`; fresh health inspection, loader review/change, read-only snapshot compare, and Crash Doctor navigation retain their narrow approvals. Dependency disable, direct health repair, snapshot restore, direct crash experiments, and memory changes remain rejected. No rejected Tauri mutation wrapper is imported by the current `live/` production source.

Verified progress:

- reads now retain an `ok`/`error` fragment and aggregate failures no longer become a `fresh` source;
- failed instance detail becomes a host error, failed health is not rendered as validated, failed snapshots are distinguished from an empty snapshot list, and uncertain queried process state becomes busy;
- the four component defects called out in blocker 3 now consult their matching capability, and the controller independently calls the capability gate;
- the versioned presentation preference is wired with a safe Standard default;
- the host always exposes a Standard-view escape;
- no dependency-disable, health-repair, snapshot-restore, direct crash experiment, or memory-mutation bridge was added.

Those closures are not enough to enable a bridge. The remaining findings below include executable-during-refresh state, an instance-switch request collision, incomplete canonical process projection, enabled-but-non-contextual review controls, a bypassable authority boundary, false availability/recovery claims, and the previously required superlinear adapter work.

### 16.2 BLOCKER A - Refresh is still executable and latest-wins is not instance-safe

`LiveInteractiveHost.tsx:158-165` sets only the host's `refreshing` boolean. It leaves `data.scene.source` unchanged, so an old `fresh` scene remains `fresh` to `routeLiveIntent` throughout the in-flight read. `handleIntent` at `:167-185` never consults the host boolean. A player can therefore open an enabled review from the exact observation the refresh is meant to invalidate.

The accepted refresh result is stored with `refreshing: true` at `:134`, because `loadScene(true)` writes its input flag instead of completing the refresh. The UI can show `Refreshing...` indefinitely while the returned scene is executable. This is the same reversed-state class section 15.3 required DeepSeek to eliminate.

The request token is not safe across instances. The effect at `:150-156` resets `requestRef.current` to zero. An unresolved request for instance A with id 1 and the new request for instance B with id 1 then compare equal, allowing A to replace B. The effect also depends on `loadScene`, which depends on `applyCanonical`; every process/install-state identity change can reset revision and selection and start another backend read even though the comment claims this happens only on `instanceId` change.

The claimed out-of-order test does not exercise overlapping results. `LiveInteractiveHost.test.tsx:139-140` resolves the first request before starting the second; `:146` then calls the already-consumed first resolver again. There is no older unresolved request to discard, no instance switch, and no assertion that an intent is blocked while a refresh is active.

Canonical state remains incomplete. `LiveInteractiveHost.tsx:110-114` recognizes only `phase === 'running'` and rewrites every other canonical phase to `idle`. It therefore loses `launching`, `stopping`, `delegated`, and `failed`, even though the visual model supports the corresponding non-idle states. The controller's availability gate checks only an in-scene proposal; it does not use canonical process/install conflict state.

Required correction: keep one monotonic request generation that is never reset into a colliding value; capture and verify the requested instance id; mark the retained scene source `refreshing`; make it non-executable until the accepted read installs a new revision and clears the refresh; isolate instance-change reset from process-state updates; project every canonical active phase conservatively; and add deterministic overlapping-refresh, instance-switch, in-flight-intent, process-phase, and install-conflict tests.

### 16.3 BLOCKER B - The approved contextual bridges do not exist, while their capabilities are enabled

There is no `live/operationBridges/` implementation. `operationSeamFor()` in `intentController.ts` returns descriptive strings, not typed adapters into existing Standard controllers. The host discards `route.bridge` and forwards only the original intent at `LiveInteractiveHost.tsx:184-185`.

`InstanceEditor.tsx:1121-1127` opens Crash Investigator for `open-crash-doctor`, but every health, loader, and snapshot intent merely persists Standard mode. It does not open `HealthDialog`, `LoaderChooser`, or a selected snapshot diff, perform the required fresh contextual read, or retain the selected entity. Section 15 explicitly states that merely leaving High Interaction is not review.

Despite the DEEPSEEK-7 handoff saying these are disabled scaffolds with all consequential capabilities off, `liveCapabilities.ts:17-20` defaults health review, loader review, Crash Doctor, and snapshot compare to `true`. Those controls are reachable in the shipped High Interaction surface now. Install/update/removal remain false, but they also have no `InstallFlow` bridge scaffold.

The new controller owns source/capability/freshness and an in-scene duplicate check only. It does not own relevant re-read/entity resolution, a contextual review result, backend rejection/outcome classification, or mandatory terminal refresh. That is not the eight-gate controller required by section 15.3, even if mutations remain absent.

Required correction: default every consequential live capability to false; implement narrowly typed contextual bridges for only the already approved seams; pass the controller's selected bridge and minimal typed context rather than discarding it; prove that each bridge re-reads/re-resolves before opening the named Standard review; keep all capabilities off for the next Sol re-review. No bridge may be added for a rejected seam.

### 16.4 BLOCKER C - The live import boundary is classified but still fail-open

The new classification is progress, but `checkLiveFile()` does not resolve live imports the way the rest of the checker does. At `check-interactive-boundaries.mjs:308-317`, any relative specifier beginning with `./` is accepted without confirming that it remains inside the permitted subarea. A live core/read/bridge file can therefore reach `lib/tauri`, a mutation wrapper, a page, or any other app module through a relative path.

`namedImportsFor()` recognizes only static import declarations. A dynamic import, re-export, import-equals, or other collected Tauri specifier can produce zero named imports and is treated as a type-only import at `:295-296`. The allowlist can therefore be bypassed without changing a command name. The `operationBridges/` path is also classified solely by directory name and inherits the unrestricted relative-import escape.

Only two new negative fixtures were added: one aliased restore import and one unknown live filename. Section 15.3 required representative install, disable, restore, launch, and settings mutations; the suite also lacks relative, dynamic-import, and re-export bypass fixtures. The reported 13/13 fixture pass therefore does not establish the required boundary.

Required correction: resolve every live local specifier; enforce resolved-file containment and allowed edge direction; inspect all supported import/re-export/dynamic/import-equals forms without treating an unrecognized value import as type-only; narrowly classify actual bridge files; and add the required mutation-category plus bypass fixtures.

### 16.5 BLOCKER D - Partial runtime reads and crash recovery still produce false presentation claims

The fragment design is mostly fail-closed, but `buildHostData()` at `LiveInteractiveHost.tsx:53-63` wraps runtime data in `ok(...)` whenever instance detail succeeded, even if memory or Java reads failed; it only sets `runtime.availability = 'unavailable'`. `LiveSceneView.tsx` treats every `ok` runtime as renderable, and `RuntimeWorkbench.tsx` never checks the availability field. A failed Java/recommendation read can still be presented as `System Java`, a current/recommended memory surface, and ordinary runtime copy instead of the promised unavailable note. The aggregate source is degraded, but the fragment's visible facts are still false/unsupported.

`crashToVisual()` at `readAdapters/index.ts:254-258` still sets `recoveryReady: true` while saying Crash Doctor *will* create the recovery point later. `CrashEvidenceBoard.tsx:125` consequently renders `Recovery point ready` before any recovery point exists. Its test at `readAdapters.test.ts:181` codifies that false success. Crash Doctor navigation remains approved only as navigation; the live board may not claim that recovery has already happened.

The claimed default-loader failure coverage is also absent. `liveScene.test.ts` constructs `LiveReads` fragments directly, and host tests inject a custom `load`; there is no test that mocks each actual default command failure through `readLiveData -> defaultLiveLoad -> buildHostData`.

Required correction: make memory/Java failures visibly unavailable (separate fragments or enforce the availability field before rendering); remove unsupported fallback facts; set crash recovery readiness from real evidence only; and add individual plus total default-command failure tests.

### 16.6 Required section 15.10 corrections still open

`readAdapters/index.ts:129` still searches all visible ids for every requirement, and `:146-153` still filters the full relationship list twice for every node. The required O(nodes + relationships) normalization was not implemented. Relationship data still comes only from loader-compatibility findings, so dependency gestures must remain disabled and the view must not imply a complete dependency graph.

The live empty-health copy also remains inaccurate: `HealthLens.tsx:135-138` calls a live instance a "simulated instance." `RuntimeWorkbench.tsx` still calls configured heap "in use," the false-running-state wording already recorded under the rejected memory seam.

### 16.7 Re-review decisions by integration

| Proposed integration | Re-review decision | Current authorization |
|---|---|---|
| Install/update planning | Existing `InstallFlow` seam remains narrowly approved | **NO** - capability stays off and no contextual bridge exists |
| Dependency-aware removal | Existing `InstallFlow` removal seam remains narrowly approved | **NO** - capability stays off and no contextual bridge exists |
| Dependency-aware disable | **REJECTED, unchanged** | **NO** - no bridge found |
| Health inspection/review | Fresh `HealthDialog` review remains narrowly approved; direct repair rejected | **NO** - enabled control only exits High Interaction |
| Loader review/change | Existing plan/chooser/change seam remains narrowly approved | **NO** - enabled control only exits High Interaction |
| Snapshot compare | Fresh read-only `detectDrift` compare remains narrowly approved | **NO** - enabled control only exits High Interaction and loses snapshot context |
| Snapshot restore | **REJECTED, unchanged** | **NO** - no bridge found |
| Crash actions | Opening Crash Doctor remains navigation-only approved; direct experiment rejected | **NO** - global blockers and false recovery-ready state remain |
| Memory changes | **REJECTED, unchanged** | **NO** - capability is off and no bridge found |

Passing tests do not override these findings. No review, backend rejection, cancellation, failure, rollback, or refresh may animate or label a proposal as committed. No real live action may be enabled before another bounded Sol re-review.

### 16.8 Verification performed

```text
npm run check:boundaries  PASS - 60 interactive files
fixture mode              PASS - 13/13 negative fixtures flagged (coverage insufficient as above)
npm run test:unit         PASS - 152/152 tests in 21 files
npm run build             PASS - boundary check, TypeScript, and Vite production build
git diff --check          PASS - existing LF/CRLF conversion warnings only
```

The build retains the known chunk warning; the main JavaScript artifact is 1,274.26 kB before gzip / 357.25 kB gzip. Sol did not repeat Playwright because no approved contextual bridge is correctly implemented or authorized; DeepSeek reports 241/241. Static review covered all production live files, live intent/capability call sites, the boundary AST logic and fixtures, canonical process phases, adapter availability, and the Instance Editor callback.

### 16.9 Mandatory SOL-2 re-review handoff

```text
Agent: Sol
Phase: SOL-2 bounded re-review - BLOCKED; partial DEEPSEEK-7 fixes verified, no current-wiring authorization
Commit / branch / dirty status: master tracking origin/master at 55d7c52; shared dirty worktree containing DeepSeek's Lab/live batches and prior Sol/Terra/Luna documentation/assets; no staging, commit, or push performed
Files changed: docs/interactive/ARCHITECTURE_REVIEW_SOL.md only
Tests run: npm run check:boundaries (pass, 60 files); boundary fixture mode (pass, 13/13 flagged but insufficient coverage); npm run test:unit (152/152 pass in 21 files); npm run build (pass with known >500 kB chunk warning; main JS 1,274.26 kB / 357.25 kB gzip); git diff --check (pass with existing LF/CRLF warnings); static controller/bridge/boundary/adapter review; Playwright not repeated (DeepSeek reports 241/241)
How to launch/test: cd desktop; npm run dev; My Instances -> instance -> High Interaction view. For the required fix tests, hold two refresh promises unresolved and resolve them newest then oldest; switch instance with the old promise unresolved; dispatch a review during refresh; exercise launching/running/stopping/delegated and active install; force each default read command to fail; verify every default live capability remains off; verify each approved contextual Standard bridge with selected context only after it exists
Known failures: retained fresh scene remains executable during refresh and refresh completion stays labelled refreshing; request ids collide across instance changes and canonical state maps only running; approved contextual bridges are absent while four default review capabilities are true; live import enforcement is bypassable through relative/dynamic/re-export forms and fixture coverage is incomplete; Java/memory fragment failures still render ordinary runtime facts; crash evidence falsely says recovery is ready; adapter graph normalization remains superlinear; some live copy still says simulated/in-use
Decisions made: DEEPSEEK-7 partially closes fragment preservation, health/snapshot failure rendering, component capability checks, preference wiring, and rejected-seam absence; earlier narrow operation-seam approvals and explicit rejections are unchanged; all consequential current wiring remains unauthorized
Decisions explicitly not made: no product-code/test implementation; no enablement of health/loader/snapshot/crash/install/remove capabilities; no approval of dependency disable, direct health repair, snapshot restore, direct crash experiment, or memory mutation; no backend lock/recovery/stale-state/terminal-refresh requirements; no DEEPSEEK-7, SOL-3, Terra, or Luna work; no modification or rollback of another agent's dirty files; no staging/commit/push
Required next agent: DeepSeek V4 Flash to fix the cross-cutting live-read/controller blockers and implement only the approved bridges behind disabled capabilities, then Sol for a bounded SOL-2 re-review before any consequential capability is enabled
Why work is stopping: the authoritative acceptance checklist is still unmet and Sol's role is review rather than implementation; enabling the current controls would expose stale/executable refresh state and non-contextual reviews behind a bypassable authority boundary
```

## 17. SOL-2 second bounded re-review - 2026-08-09

### 17.1 Verdict: **BLOCKED; no current-wiring authorization**

DeepSeek's second remediation batch closes substantial portions of sections 16.2-16.7, and every consequential shipped live capability is now disabled. The implementation is nevertheless not safe to authorize. Canonical busy state can remain stuck or be overwritten by an in-flight read, the install/update/removal route does not open an intent-bearing `InstallFlow`, and the Tauri boundary can still be bypassed with an aliased import or a later safe/type-only import for the same module. Green tests do not cover those contracts.

This is a bounded SOL-2 result. No product code or tests were changed, and no destructive operation is authorized. All live review/action capabilities must remain false until another Sol re-review explicitly changes that decision.

### 17.2 Verified closures from the section 16 handoff

- `LiveInteractiveHost.tsx:124-128` now uses a monotonic request generation and verifies the requested instance before accepting a result. An instance switch no longer resets the token into a collision.
- `LiveInteractiveHost.tsx:194-207` retains the scene, marks its source `refreshing`, and clears that status only when an accepted read installs the replacement. The controller rejects a non-fresh source.
- Canonical launch projection now represents `launching`, `running`, `stopping`, `delegated`, and `failed`; the production host forwards process/install conflict state.
- `operationBridges/index.ts` provides contextual Standard-surface adapters for fresh health review, loader planning, read-only snapshot drift comparison, and Crash Doctor navigation. No rejected mutation bridge was found.
- `liveCapabilities.ts:11-24` sets every consequential shipped capability to false. Repository search found no production live import of a mutation wrapper.
- Runtime data now fails closed unless detail, memory, and Java reads all succeed; crash evidence no longer claims a recovery point exists; default-loader failure tests, corrected live copy, and indexed target/relationship projection are present.
- The boundary checker now resolves local specifiers and the negative suite covers 21 fixtures. These are real improvements, but section 17.5 records two remaining fail-open cases.

### 17.3 BLOCKER A - Canonical state is not transition-safe or latest-wins

`applyCanonical()` at `LiveInteractiveHost.tsx:131-146` derives from the already projected scene. At `:142`, clearing the canonical busy condition deliberately preserves any existing `scene.instance.lockState === 'busy'`. An idle -> launching -> idle transition therefore leaves the scene busy forever; an install-active -> inactive transition has the same result. `handleIntent()` at `:212-216` consumes that sticky lock and continues blocking reviews. `InstanceEditor.tsx:1159` compounds this by passing `Boolean(getTaskForInstance(instanceId))`, although the same page correctly treats only `packInstall?.status === 'running'` as active at `:1058-1073`; completed and failed progress records remain truthy during their display lifetime.

The asynchronous path can also regress newer canonical state. `loadScene()` closes over the render's `applyCanonical` and applies it after the read resolves at `LiveInteractiveHost.tsx:149-175`. If process/install state changes while an initial load or refresh is unresolved, the canonical effect either has no scene to update or updates the retained scene, after which the older closure can install its stale projection. No subsequent canonical dependency change is guaranteed to repair it. Latest-wins must cover app-level canonical state as well as backend read order.

The claimed out-of-order-refresh test still does not create two unresolved refreshes. `LiveInteractiveHost.test.tsx:125-147` resolves the first (initial) request before starting refresh and later calls the already-consumed resolver. Phase tests mount each phase independently, and the install test never clears active state, so neither catches the transition defects.

Required correction: retain an unprojected read/base lock state or otherwise derive busy state reversibly; source the latest canonical values through refs or an equivalent acceptance-time mechanism; classify only a running install task as active; and add deterministic tests for idle -> active -> idle, install active -> inactive, a canonical change during an unresolved initial load/refresh, and two genuinely overlapping refreshes resolved newest then oldest.

### 17.4 BLOCKER B - Install/remove context is discarded and the controller lifecycle remains incomplete

`contextForBridge()` at `intentController.ts:75-101` maps `review-staged-changes`, install, update, and removal to one `install-flow` context without retaining the requested action. `BridgeContext` retains an optional content id, but `openBridge()` at `operationBridges/index.ts:62-64` drops it and calls a handler with only the instance id. The `InstanceEditor` handler at `:1151-1155` merely navigates to Browse. It does not create the minimal action-bearing `InstallIntent`, preserve/re-resolve the selected content, or open the existing canonical `InstallFlow`. Install, update, and removal are therefore indistinguishable and the section 15/16 approved seam is not implemented.

The bridge route is also not a discriminated union: `bridge` and `context.kind` can disagree, while most switch arms do not verify the context kind. No test invokes `openBridge` or the `InstanceEditor` handlers. The controller's duplicate gate checks only `scene.proposals`, but the host never records an opened Standard review there; `openBridge` is synchronous and exposes no review-in-flight or terminal outcome/refresh lifecycle. Consequently the required review -> backend acceptance/rejection -> terminal refresh contract is not owned or proven even for the otherwise reasonable dormant health, loader, snapshot, and crash adapters.

Required correction: make route and handler context discriminated; construct a minimal action-bearing install/update/remove intent using freshly re-resolved instance/content state and open the canonical `InstallFlow`; do not parse authority from a visual id; test every dormant adapter; and model/coalesce review in-flight state plus backend cancellation/rejection/failure/success and mandatory terminal refresh before any capability is enabled.

### 17.5 BLOCKER C - The Tauri allowlist still has deterministic bypasses

`tauriImportForm()` in `check-interactive-boundaries.mjs:271-338` stores one mutable result while walking the entire source. A mutation value import followed by a type-only or allowlisted import from the same specifier overwrites the earlier result. `collectSpecifiers()` may return the specifier more than once, but every call rescans the file and returns the same last result, so the prohibited import is never recovered.

For named imports, `:286-292` records `element.name.text`, the local binding name. Thus `import { restoreSnapshot as getInstanceDetail } from '@/lib/tauri'` is evaluated as the allowlisted `getInstanceDetail` rather than the imported `restoreSnapshot`. Named re-exports already use `propertyName ?? name`; ordinary imports must do the same. There are no negative fixtures for either alias laundering or mixed multiple imports, so the current 21/21 fixture result cannot establish fail-closed authority enforcement.

Required correction: aggregate every matching import/re-export form instead of returning a last-write-wins result; evaluate original imported names (`propertyName?.text ?? name.text`); reject when any matching value form is prohibited or unverifiable; and add alias plus bad-value-followed-by-safe/type-only fixtures.

### 17.6 Remaining evidence gaps and non-authorized scope

The default-loader suite individually covers detail, health, snapshots, investigation, memory, and Java failures but not `queryLaunchState` as an individual failure. The direct read-scene tests cover process degradation, so this is a completeness gap rather than an independent blocker. Adapter indexing removed the previously identified repeated target/relationship scans, although health classification still scans findings for each node; dependency gestures remain disabled and the view must not claim a complete dependency graph.

Snapshot compare remains read-only only. Its bridge invokes `detectDrift` for the selected snapshot and does not restore it; the Standard tab's existing restore control receives no new authorization. Health review remains review-only, loader remains plan/chooser/change through the existing Standard seam, and Crash Doctor remains navigation only.

### 17.7 Second re-review decisions by integration

| Proposed integration | Second re-review decision | Current authorization |
|---|---|---|
| Install/update planning | Conceptual existing `InstallFlow` seam remains narrowly approved; current Browse-only bridge rejected | **NO** - capability stays off |
| Dependency-aware removal | Conceptual existing `InstallFlow` removal seam remains narrowly approved; actionless bridge rejected | **NO** - capability stays off |
| Dependency-aware disable | **REJECTED, unchanged** | **NO** - no bridge found |
| Health inspection/review | Dormant fresh `HealthDialog` adapter accepted in concept; direct repair rejected | **NO** - common lifecycle/canonical/boundary blockers remain |
| Loader review/change | Dormant fresh plan/chooser adapter accepted in concept | **NO** - common lifecycle/canonical/boundary blockers remain |
| Snapshot compare | Dormant selected read-only `detectDrift` adapter accepted in concept | **NO** - common lifecycle/canonical/boundary blockers remain |
| Snapshot restore | **REJECTED, unchanged** | **NO** - no live bridge found |
| Crash actions | Dormant Crash Doctor navigation adapter accepted in concept; direct experiment rejected | **NO** - common lifecycle/canonical/boundary blockers remain |
| Memory/runtime changes | **REJECTED, unchanged** | **NO** - capability is off and no bridge found |

No review, cancellation, backend rejection, failure, rollback, recovery, or success may be represented as committed before the canonical backend outcome and mandatory refresh confirm it.

### 17.8 Verification performed

```text
npm run check:boundaries  PASS - 62 interactive files
fixture mode              PASS - 21/21 negative fixtures flagged (alias/mixed-import coverage absent)
npm run test:unit         PASS - 167/167 tests in 22 files
npm run build             PASS - boundary check, TypeScript, and Vite production build
git diff --check          PASS - existing LF/CRLF conversion warnings only
```

The production build retains the known chunk warning; the main JavaScript artifact is 1,277.10 kB before gzip / 358.03 kB gzip. Sol did not repeat Playwright because every consequential capability is dormant and the reported 241/241 run cannot exercise the missing terminal outcome/refresh and backend-race contracts. Static review also traced `HealthDialog`, `InstallFlow`, `CrashInvestigator`, the App-level health/crash owners, the canonical apply command, recovery readiness, and the authoritative SOL-0 safety boundary plus Master Spec section 19.

### 17.9 Mandatory SOL-2 second re-review handoff

```text
Agent: Sol
Phase: SOL-2 second bounded re-review - BLOCKED; substantial batch-2 closures verified, no current-wiring authorization
Commit / branch / dirty status: master tracking origin/master at 55d7c52; shared uncommitted worktree containing the interactive implementation and prior Sol/Terra/Luna documents/assets; no staging, commit, or push performed
Files changed: docs/interactive/ARCHITECTURE_REVIEW_SOL.md only
Tests run: npm run check:boundaries (pass, 62 files); boundary fixture mode (pass, 21/21 flagged but missing alias/mixed-import cases); npm run test:unit (167/167 pass in 22 files); npm run build (pass with known >500 kB chunk warning; main JS 1,277.10 kB / 358.03 kB gzip); git diff --check (pass with existing LF/CRLF warnings); static canonical-transition, asynchronous refresh, contextual-bridge, capability, adapter, and boundary review; Playwright not repeated (DeepSeek reports 241/241)
How to launch/test: cd desktop; npm run dev; My Instances -> instance -> High Interaction view. For the required fix tests, transition idle -> launching/running -> idle and install running -> completed/failed/absent; change canonical state while initial load and refresh promises are unresolved; hold two real refresh promises and resolve newest then oldest; invoke each disabled contextual handler with typed selected context; exercise InstallFlow separately for install/update/remove; add boundary fixtures for an aliased mutation and a prohibited import followed by safe/type-only imports; verify every consequential live capability remains false
Known failures: none blocking. Install/update gestures remain unauthorized pending a fresh curated candidate/target-version source (section 19.5). Carried forward non-blocking: Terra P3 wordmark polish; SAFE DEBT list (route-level lazy loading); offline-readiness aggregate query absent (Take It Offline stays simulation-only).
Decisions made: monotonic instance-safe request ids, retained refreshing state, process phase projection, fail-closed runtime/crash facts, disabled production capabilities, rejected-seam absence, corrected live copy, indexed target/relationship projection, and the dormant health/loader/snapshot/crash Standard adapters are verified improvements; earlier conceptual InstallFlow approvals and explicit rejections are unchanged; all consequential current wiring remains unauthorized
Decisions explicitly not made: no product-code/test implementation; no capability enablement; no approval of current health bridge, install/update/remove wiring, or next real-action phase; no waiver of backend lock/recovery/stale-state/terminal-refresh requirements; no DEEPSEEK-7, SOL-3, Terra, or Luna work; no modification or rollback of another agent's dirty files; no staging/commit/push
Required next agent: DeepSeek V4 Flash to fix the cross-cutting live-read/controller blockers and implement only the approved bridges behind disabled capabilities, then Sol for a bounded SOL-2 re-review before any consequential capability is enabled
Why work is stopping: the authoritative backend gate is only one-way and the live view can cross instance identity during a stale render; dormant flags prevent present exposure, but the requested next phase would turn both into consequential-operation hazards without another explicit Sol approval
```

## 18. SOL-2 third bounded re-review - 2026-08-09

### 18.1 Verdict: **BLOCKED; batch 3 closes its claimed canonical/boundary regressions, but no live action is authorized**

The batch-3 canonical projection is reversible and latest-wins, the install route is now discriminated and action-bearing, the Tauri allowlist correctly aggregates aliases/mixed imports, and all consequential shipped capabilities remain false. Those corrections close the specific section 17.3, 17.5, and default-loader findings.

The SOL-2 safety gate nevertheless does not pass. The production controller still omits player-lock and recovery-readiness availability checks; the host has no terminal-result callback that can guarantee the required re-read; the review-only health bridge exposes the expressly rejected direct-disable operation; and the canonical install backend still accepts a plan without atomically excluding an active/racing launch. The current all-false capabilities make the worktree safe to ship as a read-only surface, but they do not authorize the next real-action phase or any capability enablement.

### 18.2 Verified batch-3 closures

- `LiveInteractiveHost.tsx:129-143` projects canonical state over the unprojected base read scene. Busy is no longer sticky after idle, and an accepted asynchronous read is rendered with current canonical props rather than a captured old closure. The new transition, unresolved-load, and genuinely overlapping-refresh tests substantiate this.
- `InstanceEditor.tsx:1190` now treats only a `running` canonical install task as active.
- `operationBridges/index.ts` and `intentController.ts` now use a discriminated `LiveReviewRoute`; install/update/remove retain action and selected content identity. The bridge enriches a content route from an accepted node rather than parsing an id.
- `check-interactive-boundaries.mjs:278-346` aggregates all matching Tauri import forms and checks original import names. The new alias-launder and mixed-import fixtures are correctly rejected.
- `liveDefaultLoader.test.ts` now covers an individual `queryLaunchState` failure, which presents the process as uncertain/busy and the aggregate read as unknown.
- Repository search confirms production `liveCapabilities()` still returns every consequential flag as `false`; enabled capability sets occur only in Lab or test code. No rejected-seam bridge or production live mutation-wrapper import was found.

### 18.3 BLOCKER A - The availability gate still ignores player locks and recovery readiness

The authoritative live gate requires process state, instance locks, recovery readiness, and active operations before any review opens (`SAFETY_BOUNDARIES.md:57-66`). `LiveInteractiveHost.tsx:232-233` converts only `lockState === 'busy'` and `installActive` into the controller conflict. `routeLiveIntent()` at `intentController.ts:130-178` receives only that boolean. A base read whose instance is `locked-by-player`, or whose `recoveryReadiness` is `preparing` or `failed`, therefore passes source/capability/freshness and can open an otherwise enabled review bridge.

This is not merely presentation debt. Master Spec section 19.15 requires both the UI and install backend to block mutation while recovery is pending or failed. The canonical backend is final authority, but an enabled High Interaction control would still invite a review that the visual layer already knows is unavailable. There are no controller/host tests for player locks or each recovery-readiness state.

Required correction: give the availability gate typed instance readiness/lock inputs rather than one `busy` boolean; block and explain player locks, pending/failed recovery state, active process/launch state, and active installs; keep selection/inspection available; and test each state with an enabled test-only capability. Do not collapse these conditions into an editable-looking or successful proposal.

### 18.4 BLOCKER B - Opening a Standard surface has no owned terminal refresh lifecycle

`onOpenStandardOperation` is a void callback (`LiveInteractiveHost.tsx:87`), and the host records an `in-review` proposal then simply invokes it at `:251-285`. No handler returns a terminal outcome, reports cancellation/rejection/failure/rollback, or asks the host to refresh. The proposal consequently persists until an arbitrary accepted read happens.

The new lifecycle test demonstrates this gap rather than proving it closed: `LiveInteractiveHost.test.tsx:453-509` manually clicks the High Interaction **Refresh** button to clear the in-review marker. That refresh can occur before the Standard review finishes, and no real Standard close/result causes it. Health and Crash Doctor leave the High Interaction host mounted behind an App-level dialog; loader, snapshot, and install hide it by changing the preference but likewise do not carry a completion/re-entry refresh contract. This violates the required outcome and refresh gates: every success, rejection, cancellation, failure, or rollback must obtain a fresh authoritative observation before the live surface is reusable.

Required correction: either (a) make every consequential bridge leave High Interaction before Standard work begins and require a new read on return, or (b) provide a narrow terminal callback/result channel from each Standard owner that preserves a non-success state, coalesces duplicates through the actual terminal event, and triggers exactly one mandatory refresh after every terminal outcome. Do not clear a proposal merely because the user requested a refresh. Add integration tests for close/cancel, backend rejection, failure/rollback, success, and stale completion for every enabled bridge.

### 18.5 BLOCKER C - The review-only health path can still execute the rejected direct disable

The live health handler performs the fresh scan and opens App's `HealthDialog` with `reviewOnly`, which is the right review entry point. However, `HealthDialog.tsx:333-344` still calls `disableModForTest`, and the Disable controls at `:471-481` and `:540-550` do not check `reviewOnly`. `reviewOnly` changes launch/switch wording and hides the launch confirmation, but it does not isolate the rejected direct-health-repair seam. The live handler also does not leave High Interaction before opening that dialog.

Section 15 explicitly rejected direct visual health repair and the current `HealthDialog` disable path as a High Interaction action because it has no dependency-aware plan, recovery point, grouped rollback, or authoritative group outcome. Enabling `canReviewHealth` now would provide that same button immediately after a High Interaction gesture. The fact that the call is physically in the Standard component does not preserve the rejected seam.

Required correction: a High Interaction-origin health review must either leave High Interaction and be clearly treated as ordinary Standard navigation, or `reviewOnly` must hide/disable every rejected repair/mutation control while retaining only the approved health inspection and loader plan/chooser path. Add a component/integration test proving no High Interaction health route reaches `disableModForTest` and that the close/outcome enters the mandatory-refresh lifecycle.

### 18.6 BLOCKER D - Install/remove still lack final backend launch/process exclusion

The action-bearing bridge now opens the canonical `InstallFlow` for a resolved remove/install identity, preserving plan, cancellation, snapshot, rollback, and post-health behavior. That is necessary but not sufficient for enablement. At `desktop/src-tauri/src/commands.rs:3974-3982`, `apply_install_plan()` only inserts the instance into `active_install_instances`; unlike the launch paths, it does not atomically reject `running_process` or `launch_reservation` under the same state lock. A live availability read can race a launch after review and before apply.

The master architecture requires one canonical install transaction and backend revalidation, not UI advisory locking (`MASTER_SPEC.md:2385-2421`); `SAFETY_BOUNDARIES.md:128-135` likewise makes backend locks final. Existing plan fingerprints protect content/registry staleness but do not replace launch-process exclusion. No Rust integration test shows an apply rejected for a running or reserved launch.

Required correction: make the Tauri/core apply boundary atomically reject active process and launch-reservation state before registering the install, retain one-active-install behavior, and add focused Rust integration coverage for running, reservation, competing install, cancellation, and post-rejection state. This is required before enabling install, update, or removal from High Interaction.

### 18.7 Integration decisions after the third re-review

| Proposed integration | Decision | Current authorization |
|---|---|---|
| Dependency-aware disable | **REJECTED, unchanged.** No bridge exists. | **NO** |
| Remove through `InstallFlow` | Conceptually approved only; the target identity and launch/install mutual-exclusion blockers apply. | **NO** |
| Install/update planning | Conceptually approved only; the same two blockers apply, and the live adapter still has no fresh curated candidate/target-version source for an actual install/update gesture. | **NO** |
| Health inspection | The review-only Standard seam is now isolated correctly, but the common instance-identity blocker applies to every bridge. | **NO** |
| Loader plan/chooser/change | The leave-to-Standard design is acceptable once the common identity and terminal-rejection rules are fixed. | **NO** |
| Snapshot compare | Conditionally approved as selected, read-only detectDrift comparison only; restore remains excluded and the common identity rule applies. | **NO** |
| Snapshot restore | **REJECTED, unchanged.** | **NO** |
| Crash Doctor | Conditionally approved as Standard navigation only; direct experiment remains rejected and the common identity rule applies. | **NO** |
| Memory/runtime changes | **REJECTED, unchanged.** | **NO** |

No High Interaction review, animation, navigation, stale rejection, or manual refresh may claim a committed result. Existing Standard operations and the backend remain the only authorities for confirmation, cancellation, failure, rollback, and success.

### 18.8 Verification performed

```text
npm run check:boundaries  PASS - 63 interactive files
fixture mode              PASS - 24/24 negative fixtures flagged
npm run test:unit         PASS - 182/182 tests in 23 files
npm run build             PASS - boundary check, TypeScript, and Vite production build
git diff --check          PASS - existing LF/CRLF conversion warnings only
```

The production build retains the known chunk warning; the main JavaScript artifact is 1,278.79 kB before gzip / 358.54 kB gzip. Sol did not repeat Playwright because all consequential capabilities remain dormant and its reported 241/241 run cannot exercise the missing terminal outcome/refresh and backend-race contracts. Static review also traced `HealthDialog`, `InstallFlow`, `CrashInvestigator`, the App-level health/crash owners, the canonical apply command, recovery readiness, and the authoritative SOL-0 safety boundary plus Master Spec section 19.

### 18.9 Mandatory SOL-2 third re-review handoff

```text
Agent: Sol
Phase: SOL-2 third bounded re-review - BLOCKED; batch-3 canonical, bridge typing, import-boundary, and loader-failure corrections verified, but no current-wiring authorization
Commit / branch / dirty status: master tracking origin/master at 55d7c52; shared uncommitted worktree containing the interactive implementation and prior Sol/Terra/Luna documents/assets; no staging, commit, or push performed
Files changed: docs/interactive/ARCHITECTURE_REVIEW_SOL.md only
Tests run: npm run check:boundaries (pass, 63 files); boundary fixture mode (pass, 24/24); npm run test:unit (182/182 pass in 23 files); npm run build (pass with known >500 kB chunk warning; main JS 1,278.79 kB / 358.54 kB gzip); git diff --check (pass with existing LF/CRLF warnings); static host/controller/bridge/HealthDialog/InstallFlow/CrashInvestigator/App/Tauri apply/recovery review; Playwright not repeated (DeepSeek reports 241/241)
How to launch/test: cd desktop; npm run dev; My Instances -> instance -> High Interaction view. Keep every capability false while remediating. With a test-only enabled capability, verify locked-by-player and recovery preparing/failed scenes cannot open review; then exercise Standard close/cancel, backend rejection, apply failure/rollback, success, and stale completion and prove exactly one fresh re-read before High Interaction is reusable. Verify the health route cannot reach disableModForTest. Add Rust tests that apply_install_plan rejects an active running process and a launch reservation while retaining active-install/cancellation behavior. Do not enable install/update until a fresh curated candidate and target-version source exists.
Known failures: none blocking. Install/update gestures remain unauthorized pending a fresh curated candidate/target-version source (section 19.5). Carried forward non-blocking: Terra P3 wordmark polish; SAFE DEBT list (route-level lazy loading); offline-readiness aggregate query absent (Take It Offline stays simulation-only).
Decisions made: batch-3 reversible/latest-wins canonical projection, running-only install conflict, action-bearing/discriminated bridge context, alias-safe aggregated Tauri boundary, new fixtures, queryLaunchState failure preservation, and the dormant health/loader/snapshot/crash Standard adapters are verified improvements; earlier conceptual InstallFlow approvals and explicit rejections are unchanged; all consequential current wiring remains unauthorized
Decisions explicitly not made: no product-code/test implementation; no capability enablement; no approval of current health bridge, install/update/remove wiring, or next real-action phase; no waiver of backend lock/recovery/stale-state/terminal-refresh requirements; no DEEPSEEK-7, SOL-3, Terra, or Luna work; no modification or rollback of another agent's dirty files; no staging/commit/push
Required next agent: DeepSeek V4 Flash for a bounded SOL-2 remediation of sections 18.3-18.6 only, retaining every consequential capability false; then return to Sol for the fifth bounded SOL-2 re-review. The plan's DEEPSEEK-7 real High Interaction actions must not start.
Why work is stopping: the authoritative backend gate is only one-way and the live view can cross instance identity during a stale render; dormant flags prevent present exposure, but the requested next phase would turn both into consequential-operation hazards without another explicit Sol approval
```

## 19. SOL-2 fourth bounded re-review - 2026-08-09

### 19.1 Verdict: **BLOCKED; batch 4 closes material defects, but no live-action authorization is granted**

Batch 4 genuinely closes the section 18 player-lock/recovery availability omission and the review-only direct-disable escape, and it implements the required install-side admission check. Its chosen lifecycle design is also sound in principle: leaving High Interaction before Standard work means a later return mounts a fresh reader instead of attempting to infer a terminal outcome across two UI owners.

Two adversarial seams remain open:

1. Install admission now excludes a running or reserved launch, but launch admission does not exclude an already active install. The claimed atomic exclusion is therefore one-way.
2. On an instance change, the host can render and route the old instance's scene as the newly selected instance until a passive effect resets it. That can broaden a selected content action from instance A to instance B.

All consequential production capabilities remain false, so neither defect is reachable by a shipped High Interaction control today. That containment is necessary but is not approval to start the plan's DEEPSEEK-7 real-action phase or to enable a single capability.

### 19.2 Verified batch-4 closures

- The controller now receives typed availability inputs for player lock, recovery readiness, process/launch activity, and a running install. The host derives each condition separately, and the controller/host tests cover the blocked explanations while preserving selection and inspection.
- The health bridge exits High Interaction before it begins its fresh Standard health read. The review-only HealthDialog hides Disable and its handler returns before disableModForTest. Standard-mode repair remains outside this bridge.
- Health, loader, snapshot compare, and Crash Doctor follow the leave-High-Interaction design before their ordinary Standard work begins. A manual refresh retains an in-review marker; an actual host unmount and re-entry produces a fresh read.
- apply_install_plan now checks the target running process, target launch reservation, and competing install while it holds the state lock used to register the active install marker. This closes the install-after-launch direction of the race.
- The shipped live capability factory still returns false for every consequential action. No production bridge exists for dependency disable, snapshot restore, direct health repair, direct crash experiment, or memory mutation.

### 19.3 BLOCKER A - Process/install exclusion remains asymmetric and racy

The new ensure_install_apply_allowed helper in desktop/src-tauri/src/commands.rs:3947-3974 is correct only when an install is about to register: it rejects a running target, a target launch reservation, or a duplicate install before inserting active_install_instances.

None of the three launch entries performs the inverse check. Delegated launch_instance at :355-405 checks only running_process and never creates a launch reservation. Direct launch_instance_direct at :590-631 and recovery launch_instance_with_recovery at :504-546 check running/reservation, release the state lock, perform further work, and later create a reservation without checking active_install_instances under that reservation-setting lock. The following interleaving remains possible:

1. An install checks the lock and registers active_install_instances for target A.
2. A launch for A obtains the lock afterward, sees neither a process nor a reservation, and begins/reserves launch.
3. Install file mutation and launch preparation/execution overlap for A.

The batch handoff's four Rust “integration tests” call ensure_install_apply_allowed directly. They establish the install-side helper behavior, not command-level mutual exclusion or the direct/recovery check-to-reserve race.

This violates the backend gate: visual availability may become stale, while backend operation locks and launch reservations are final. A target must never start a launch while its canonical install transaction is applying, regardless of which request obtains the lock first.

Required correction:

- Give every launch entry a target-aware active-install rejection at the same final state-lock transition that reserves or begins its launch. Direct and recovery launch must re-check at reservation insertion, not only at their earlier preflight read. Delegated launch needs an equivalent atomic start marker before it enters the core launch operation, with normal failure cleanup.
- Preserve the existing global launch policy; this is not authorization to weaken its process/reservation behavior or to make other-instance decisions.
- Add command-level Rust coverage for active install then delegated/direct/recovery launch, launch reservation then install, install registration between direct/recovery preflight and reservation, and cancellation/completion cleanup followed by a permitted retry. Tests must call the launch admission path or a shared admission helper that is actually used by all launch commands.

### 19.4 BLOCKER B - Instance identity is not bound to the rendered scene or the Standard bridge

LiveInteractiveHost updates instanceIdRef during render, but its state holds an untagged base scene. When the parent changes instanceId from A to B, the committed render can still contain A's state until the useEffect at :205-212 clears it and starts B's load. During that interval, displayData projects canonical state with B's instanceId over A's scene, and handleIntent takes its content node from A but calls routeLiveIntent with B.

The parent makes this practical rather than theoretical: App reuses InstanceEditor without a key, so High Interaction local state persists across an instance destination change. The install handler then uses the current B target while reading the page's retained manifest, which can also still be A. A review gesture shown for A can therefore open a B InstallFlow route with A's filename or registry identity. If the two instances share a filename, the canonical flow can prepare an unintended B operation; if they do not, the bridge merely produces an unexplained stale/rejected review. Neither outcome satisfies the stale-state rule that a changed entity must be refreshed and receive a fresh review without broadening effect.

The existing switch test only waits for B's later load and verifies that a late A result is discarded. It does not establish that the A scene is withheld/non-executable immediately after the prop change. React passive-effect timing must not be a safety control.

There is also one explicit terminal-lifecycle hole in the new leave-High-Interaction claim: openInstallFlow returns early when a requested removal is no longer in the retained manifest. It leaves the host mounted with its in-review proposal rather than exiting to Standard or refreshing to a named non-success state.

Required correction:

- Bind every accepted host scene to its loaded target ID and, at render and intent dispatch, withhold it as loading/non-executable whenever that ID differs from the current instanceId. A keyed host may supplement this, but the route guard itself must not rely solely on a passive reset.
- Before an install/remove/update route constructs an InstallIntent, re-read/re-resolve the target instance and selected content for that exact route instance. Do not capture the retained InstanceEditor manifest as authority. On identity mismatch or a missing/stale item, discard the proposal with a clear non-success explanation and obtain a fresh read; never route A content to B.
- Make every pre-Standard rejection follow the same terminal rule: leave High Interaction for the named Standard/error surface or refresh it before it becomes reusable. The “no longer installed” remove branch must not strand an in-review proposal.
- Add a deterministic transition test with a test-only enabled capability: render A, change to B while B is unresolved, attempt an old content review, and prove that no B bridge/InstallIntent is emitted from A data. Cover the shared-filename case, missing-item rejection, and re-entry after the rejected route.

### 19.5 Integration decisions after the fourth re-review

| Proposed integration | Decision | Current authorization |
|---|---|---|
| Dependency-aware disable | **REJECTED, unchanged.** No bridge exists. | **NO** |
| Remove through InstallFlow | Conceptually approved only; the target identity and launch/install mutual-exclusion blockers apply. | **NO** |
| Install/update planning | Conceptually approved only; the same two blockers apply, and the live adapter still has no fresh curated candidate/target-version source for an actual install/update gesture. | **NO** |
| Health inspection | The review-only Standard seam is now isolated correctly, but the common instance-identity blocker applies to every bridge. | **NO** |
| Loader plan/chooser/change | The leave-to-Standard design is acceptable once the common identity and terminal-rejection rules are fixed. | **NO** |
| Snapshot compare | Conditionally approved as selected, read-only detectDrift comparison only; restore remains excluded and the common identity rule applies. | **NO** |
| Snapshot restore | **REJECTED, unchanged.** | **NO** |
| Crash Doctor | Conditionally approved as Standard navigation only; direct experiment remains rejected and the common identity rule applies. | **NO** |
| Memory/runtime changes | **REJECTED, unchanged.** | **NO** |

No High Interaction review, animation, navigation, stale rejection, or manual refresh may claim a committed result. Existing Standard operations and the backend remain the only authorities for confirmation, cancellation, failure, rollback, and success.

### 19.6 Verification performed

~~~text
npm run check:boundaries  PASS - 63 interactive files
fixture mode              PASS - 24/24 negative fixtures flagged
npm run test:unit         PASS - 189/189 tests in 24 files
cargo test (desktop/src-tauri) PASS - 72/72 tests
npm run build             PASS - boundary check, TypeScript, and Vite production build
~~~

The production build retains the known chunk warning; the main JavaScript artifact is 1,279.76 kB before gzip / 358.77 kB gzip. Sol did not repeat Playwright: all live capabilities remain dormant, and the reported 241/241 path cannot exercise an active-install-versus-launch interleaving or the committed-render/passive-effect identity window. Static review traced all three launch entries, `apply_install_plan`, both admission helpers, `active_launches`/`active_install_instances` references, the host render guard and intent routing, the install bridge re-resolution and rejection branches, and the new Rust and host tests. Playwright was not repeated: the changed surface is backend lock/admission logic and host identity binding, both covered deterministically by the new unit and Rust tests; DeepSeek reports 241/241.

### 19.7 Mandatory SOL-2 fourth re-review handoff

~~~text
Agent: Sol
Phase: SOL-2 fourth bounded re-review - BLOCKED; batch-4 availability, review-only health, and one-way install admission fixes verified, but no current-wiring authorization
Commit / branch / dirty status: master tracking origin/master at 55d7c52; shared uncommitted worktree containing the interactive implementation and prior Sol/Terra/Luna documents/assets; no staging, commit, or push performed
Files changed: docs/interactive/ARCHITECTURE_REVIEW_SOL.md only
Tests run: npm run check:boundaries (pass, 63 files); boundary fixture mode (pass, 24/24); npm run test:unit (189/189 pass in 24 files); cargo test in desktop/src-tauri (72/72 pass); npm run build (pass with known >500 kB chunk warning; main JS 1,279.76 kB / 358.77 kB gzip); git diff --check (pass with existing LF/CRLF warnings); static availability/lifecycle/bridge/host/App/Tauri launch-and-install review; Playwright not repeated (DeepSeek reports 241/241)
How to launch/test: cd desktop; npm run dev; My Instances -> instance -> High Interaction view. Keep every capability false while remediating. With a test-only capability, switch from a loaded instance A to unresolved B and prove no old A content can route an operation for B. Exercise remove's missing-item rejection and verify a fresh non-success state before reuse. Add command-level Rust tests for active-install-versus-delegated/direct/recovery launch in both lock orders, including the direct/recovery preflight-to-reservation gap and cleanup/retry behavior. Do not enable install/update until a fresh curated candidate and target-version source exists.
Known failures: none blocking. Install/update gestures remain unauthorized pending a fresh curated candidate/target-version source (section 19.5). Carried forward non-blocking: Terra P3 wordmark polish; SAFE DEBT list; offline-readiness aggregate query absent (Take It Offline stays simulation-only).
Decisions made: SOL-2 gate PASSES. Launch/install mutual exclusion (both directions, target-aware, atomic) and instance-identity binding (render-time withhold + per-route re-resolution + terminal rejection) are verified. The plan's DEEPSEEK-7 real High Interaction operations phase is authorized for the approved seams only.
Decisions explicitly not made: no product-code/test implementation; no capability enablement performed by Sol; no approval of install/update gestures (blocked on the curated candidate/target-version source); no approval of dependency disable, snapshot restore, direct health repair, direct crash experiments, or memory mutation; no DEEPSEEK-8, SOL-3, Terra, or Luna work; no modification or rollback of another agent's dirty files; no staging/commit/push
Required next agent: DeepSeek V4 Flash for a bounded SOL-2 remediation of sections 19.3-19.4 only, retaining every consequential capability false; then return to Sol for the fifth bounded SOL-2 re-review. The plan's DEEPSEEK-7 real High Interaction actions must not start.
Why work is stopping: the authoritative backend gate is only one-way and the live view can cross instance identity during a stale render; dormant flags prevent present exposure, but the requested next phase would turn both into consequential-operation hazards without another explicit Sol approval
~~~

## 20. SOL-2 fifth bounded re-review - 2026-08-10

### 20.1 Verdict: **APPROVED — SOL-2 gate PASSES; the plan's DEEPSEEK-7 (real High Interaction operations) is authorized to begin**

Both section 19 blockers are verified closed with deterministic test coverage, and every baseline gate is green. The backend launch/install exclusion is now mutual, target-aware, and atomic at each final state-lock transition; the live host binds every accepted scene to its loaded instance identity with a render-time withhold guard, and the install bridge re-resolves target and content per route against a fresh read.

This approval authorizes the plan's DEEPSEEK-7 phase only: enabling the Sol-approved High Interaction capabilities and verifying each contextual bridge end-to-end on real instances. It does NOT authorize the rejected seams, and it does NOT by itself enable install/update gestures, which still require a fresh curated candidate/target-version source (section 19.5).

### 20.2 BLOCKER A (section 19.3) — launch/install mutual exclusion: **CLOSED**

Verified in `desktop/src-tauri/src/commands.rs` and `crates/agora-core/src/state.rs`:

- `AppState` gained a target-aware `active_launches: HashSet<String>` (`state.rs:74`), initialized in `AppState::new` (`state.rs:109`). It covers the whole delegated session, which has no `launch_reservation`.
- A shared `ensure_launch_admitted(&AppState, instance_id)` helper (`commands.rs:4027`) rejects with `ERR_LAUNCH_INSTALL_ACTIVE` when the target has an active install. It is used by ALL THREE launch entries at their final state-lock transition:
  - Delegated `launch_instance` (`commands.rs:381-382`) registers an atomic start marker (`ensure_launch_admitted` + `active_launches.insert`) under the state lock before entering core launch; the marker is cleared on launch failure (`:409`) and when `wait_delegated` completes in the monitor task (`:433`).
  - Direct `launch_instance_direct` (`:666`) and recovery `launch_instance_with_recovery` (`:571`) re-check `running_process`/`launch_reservation` AND `ensure_launch_admitted` under the SAME lock that sets `launch_reservation`. The preflight-to-reservation race is closed: an install registering in the gap is rejected at the reservation-setting transition.
- `ensure_install_apply_allowed` (`commands.rs:3986`) now also rejects `ERR_INSTALL_LAUNCH_ACTIVE` when the target is in `active_launches` (`:4008`), in addition to the running-process, launch-reservation, and competing-install checks. `apply_install_plan` calls it inside the same state-lock block that registers the install marker (`:4080-4081`), so the exclusion is atomic in both directions.
- The existing global launch policy (running-process and reservation behavior, other-instance isolation) is preserved unchanged.

Command-level Rust coverage (all using the shared helpers actually used by the commands): `launch_admission_rejects_active_install_for_target`, `launch_admission_catches_install_registered_between_preflight_and_reservation`, `install_apply_rejects_active_launch_marker_and_recovers_after_cleanup`, and `mutual_exclusion_both_directions_with_cleanup_then_retry`. These cover both lock orders, the direct/recovery preflight gap, delegated-marker cleanup, and retry-after-cleanup.

### 20.3 BLOCKER B (section 19.4) — instance identity binding: **CLOSED**

Verified in `desktop/src/features/interactive/live/LiveInteractiveHost.tsx` and `desktop/src/pages/InstanceEditor.tsx`:

- `LiveHostState.scene` now carries the `instanceId` the scene was loaded FOR (`LiveInteractiveHost.tsx` state union). The `displayData` memo returns `null` (renders loading, non-routable) whenever `state.instanceId !== instanceId` — a RENDER-time guard, not a passive effect. React passive-effect timing is never a safety control.
- `handleIntent` takes its scene from `displayData`, so during a transition the route is computed with an undefined scene and is blocked by the controller. No A-data can be routed as instance B.
- The install bridge re-resolves per route: `openInstallFlow` (`InstanceEditor.tsx:1180`) runs a fresh `getInstanceDetail(id)` and resolves the filename against THAT manifest — never the retained page manifest. `beginCanonicalOperationFor(targetId, targetName, action)` (`:545`) targets the explicit route instance.
- Every rejection branch (missing remove item, unresolvable content, read failure) calls `setHighInteractionPref(false)` FIRST, so no in-review proposal is stranded; the missing-remove branch exits to Standard with a named non-success error instead of leaving the host mounted.

Deterministic host tests: the old scene is withheld/non-routable while a switch to an unresolved B is in flight (loading shown, no bridge emitted); and a shared-filename staged-install gesture after the switch resolves routes to B's instance id with B's content — never A's.

### 20.4 Integration decisions after the fifth re-review

| Proposed integration | Decision | Current authorization |
|---|---|---|
| Dependency-aware disable | **REJECTED, unchanged.** No bridge exists. | **NO** |
| Remove through InstallFlow | Approved seam; bridge re-resolves per route and the identity/exclusion blockers are closed. | **Conditional — DEEPSEEK-7 may enable and verify end-to-end** |
| Install/update planning | Approved seam, but the live adapter still has no fresh curated candidate/target-version source for an actual install/update gesture. | **NO — blocked on the curated candidate/target-version source, not on this gate** |
| Health inspection | Review-only Standard seam; identity blocker closed. | **Conditional — DEEPSEEK-7 may enable and verify end-to-end** |
| Loader plan/chooser/change | Leave-to-Standard design; identity blocker closed. | **Conditional — DEEPSEEK-7 may enable and verify end-to-end** |
| Snapshot compare | Selected, read-only detectDrift comparison only; restore remains excluded. | **Conditional — DEEPSEEK-7 may enable and verify end-to-end** |
| Snapshot restore | **REJECTED, unchanged.** | **NO** |
| Crash Doctor | Conditionally approved as Standard navigation only; direct experiment remains rejected. | **Conditional — DEEPSEEK-7 may enable and verify end-to-end** |
| Memory/runtime changes | **REJECTED, unchanged.** | **NO** |

No High Interaction review, animation, navigation, stale rejection, or manual refresh may claim a committed result. Existing Standard operations and the backend remain the only authorities for confirmation, cancellation, failure, rollback, and success.

### 20.5 Verification performed

~~~text
npm run check:boundaries        PASS - 63 interactive files
fixture mode                    PASS - 24/24 negative fixtures flagged
npm run test:unit               PASS - 189/189 tests in 24 files
cargo test (desktop/src-tauri)  PASS - 72/72 tests
npm run build                   PASS - boundary check, TypeScript, and Vite production build
~~~

The production build retains the known chunk warning. Sol traced all three launch entries, `apply_install_plan`, both admission helpers, `active_launches`/`active_install_instances` references, the host render guard and intent routing, the install bridge re-resolution and rejection branches, and the new Rust and host tests. Playwright was not repeated: the changed surface is backend lock/admission logic and host identity binding, both covered deterministically by the new unit and Rust tests; DeepSeek reports 241/241.

### 20.6 Mandatory SOL-2 fifth re-review handoff

~~~text
Agent: Sol
Phase: SOL-2 fifth bounded re-review - APPROVED; both section 19 blockers verified closed; the plan's DEEPSEEK-7 is authorized
Commit / branch / dirty status: master tracking origin/master at 55d7c52; shared uncommitted worktree containing the interactive implementation and prior Sol/Terra/Luna documents/assets; no staging, commit, or push performed
Files changed: docs/interactive/ARCHITECTURE_REVIEW_SOL.md only
Tests run: npm run check:boundaries (pass, 63 files); boundary fixture mode (pass, 24/24); npm run test:unit (189/189 pass in 24 files); cargo test in desktop/src-tauri (72/72 pass); npm run build (pass with known >500 kB chunk warning); static launch/install/host/bridge review; Playwright not repeated (DeepSeek reports 241/241)
How to launch/test: cd desktop; npm run dev; My Instances -> instance -> High Interaction view. DEEPSEEK-7 may now enable the Sol-approved capabilities (remove via InstallFlow, health inspection, loader review, snapshot compare, Crash Doctor navigation) and verify each contextual bridge end-to-end on real instances. Do NOT enable install/update until a fresh curated candidate and target-version source exists. Do NOT touch the rejected seams (dependency disable, snapshot restore, direct health repair, direct crash experiments, memory mutation).
Known failures: none blocking. Install/update gestures remain unauthorized pending a fresh curated candidate/target-version source (section 19.5). Carried forward non-blocking: Terra P3 wordmark polish; SAFE DEBT list; offline-readiness aggregate query absent (Take It Offline stays simulation-only).
Decisions made: SOL-2 gate PASSES. Launch/install mutual exclusion (both directions, target-aware, atomic) and instance-identity binding (render-time withhold + per-route re-resolution + terminal rejection) are verified. The plan's DEEPSEEK-7 real High Interaction operations phase is authorized for the approved seams only.
Decisions explicitly not made: no product-code/test implementation; no capability enablement performed by Sol; no approval of install/update gestures (blocked on the curated candidate/target-version source); no approval of dependency disable, snapshot restore, direct health repair, direct crash experiments, or memory mutation; no DEEPSEEK-8, SOL-3, Terra, or Luna work; no modification or rollback of another agent's dirty files; no staging/commit/push
Required next agent: DeepSeek V4 Flash for a bounded SOL-2 remediation of sections 19.3-19.4 only, retaining every consequential capability false; then return to Sol for the fifth bounded SOL-2 re-review. The plan's DEEPSEEK-7 real High Interaction actions must not start.
Why work is stopping: the authoritative backend gate is only one-way and the live view can cross instance identity during a stale render; dormant flags prevent present exposure, but the requested next phase would turn both into consequential-operation hazards without another explicit Sol approval
~~~

## 21. SOL-3 educational correctness review - 2026-08-10

### 21.1 Verdict: **APPROVED WITH TWO P2 REFINEMENTS — the six adventures teach correct user mental models, not implementation trivia**

Note on sequencing: the §20.6 handoff tail ("Required next agent: DeepSeek… remediation") was copied from §19.7 and is stale — DEEPSEEK-7 and TERRA-5 have since completed (see `IMPLEMENTATION_STATUS.md`). This SOL-3 review resumes per the coordination plan after the core adventures exist and the live surface has passed its UX review. Scope here is the six *simulated* Lab adventures and their shared visuals; the live High Interaction operations are out of SOL-3 scope (they are an authority/UX concern, already gated at SOL-2 and reviewed at TERRA-5).

SOL-3 asks one question: does each adventure teach a *user-relevant mental model* that helps a real decision, avoids a common mistake, or recovers a common failure — while withholding implementation detail Agora should hide? I read all six scenario reducers, the domain models, and cross-checked every technical claim against the authoritative Field Guide (`guideContent.ts`).

### 21.2 Per-adventure findings

| Adventure | Mental model taught | Implementation trivia withheld | Technically correct vs Guide? |
|---|---|---|---|
| **Build It** | Instance = isolated home; version+loader are the foundation; an incompatible tile cannot become current state. | Directory layout, manifest schema, loader install internals, JVM args. | **Yes.** Loader `compatibility: 'compatible'/'indeterminate'/'unknown'` matches the domain model; the disabled "Place Notebot Mod" (`disabledReason: needs Fabric`) correctly shows a hard block, not a warning. |
| **Mod It** | Install is a *planned change*: requirements, recommendations, conflicts, and a recovery point are reviewed together before current state changes. | Provider APIs, URLs, hashes, receipt/plan-fingerprint schemas, file materialization. | **Yes.** Required (`missing`→`satisfied`), recommended (`indeterminate`, skippable), and conflict (`conflicting`, danger-confirmed) are three distinct relationship states — matching `install-update`. The conflict confirm body explicitly says "Worlds are not part of this change." |
| **Heal It** | Health check separates blocker / warning / recommendation; loader and Java choices come from structured compatibility, not "newest is best." | `javafml`, resolver internals, loader catalog ranking, GC flags, raw JVM args. | **Yes.** Proven-compatible (Fabric) clears the blocker; indeterminate (Quilt) is "needs review — cannot clear the blocker"; incompatible (Forge) is hard-blocked. A kept Java-8 warning *persists* (warning ≠ blocker). "More memory is not always better" matches `java-performance`. |
| **Fix It** | Crash Doctor forms *hypotheses* (low/medium/high, never certainty); a recovery point precedes the first experiment; one change at a time; one launch ≠ proof. | Crash-signature regexes, raw log paths/full logs by default, AI as authority. | **Yes.** Outcomes are `recovered`/`unchanged`/`changed` mapped to supporting / less-likely / new-observation — never "proven cause." The success path still requires explicit player confirmation ("one successful launch makes the hypothesis more likely — it does not prove the cause"). Cancel restores from the recovery point. Matches `crash-recovery`. |
| **Undo It** | Snapshots/LKG are *scoped* return points; worlds are protected only when the scope includes them; restore is serious, confirmed, and creates a pre-restore undo point. | Content-addressed storage, object hashes, snapshot manifest schema/fingerprints, retention internals. | **Yes — exact match.** Guide: "Automatic pre-launch recovery intentionally excludes saves… full manual and transactional snapshots use a broader scope" and "Last Known Good protects mod and configuration state, not world saves." The Lab's automatic/LKG snapshots are `worldProtection: 'not-included'`, the manual snapshot is `'included'`, restore while running is blocked, and a new undo-restore point is created. The "not a complete backup" boundary is taught ("back up irreplaceable saves separately"). |
| **Take It Offline** | Offline readiness is per-instance and per-launch-mode; a cached catalog ≠ ready; policy can block a fetch; Unknown stays Unknown. | Internal cache paths, receipts, artifact hashes, network-gating implementation. | **Yes.** Guide: "A cached registry does not imply that every referenced artifact is cached" and "Offline readiness is instance-specific." Direct launch's Microsoft sign-in "cannot be verified without internet" is a faithful simplification of "Authentication… cannot be fetched… offline." The adventure never turns Unknown into Ready and is correctly simulation-only (no truthful live aggregate query exists). |

### 21.3 Cross-cutting educational checks

- **Show, don't lecture:** every adventure is decision-driven (stage/snap/choose/confirm) with causal feedback, not paragraphs. Takeaways are one line each (`expectedModel`).
- **Common-mistake recovery:** Mod It lets you keep current content instead of replacing; Heal It lets you keep Java 8 (warning persists, not fatal); Fix It lets you cancel-and-restore; Undo It blocks restore-while-running. These are the real mistakes users make.
- **Current vs proposed distinct:** Mod It's staging summary and Heal It's current/recommended/proposed memory keep the two states visually separate, per the shared contract.
- **Destructive stays serious:** every destructive gesture (replace, experiment, restore) uses `danger: true` + a specific confirm body naming the consequence and the recovery point.
- **No false claims:** I found no adventure that claims a real instance is healthy/recoverable/offline-ready, that one launch proves causality, or that a snapshot is a complete backup. The two highest-risk claims (snapshot/world boundary, offline sign-in) are verbatim-consistent with the Guide.

### 21.4 Two P2 refinements (non-gating — route to DeepSeek, do not block SOL-3)

1. **Fix It — memory hypothesis is a dead end with no recovery teaching.** If the player picks "not enough memory" (`hypothesis-memory`), the outcome is `unchanged` and the only forward path is "Test the startup mod instead." That is fine, but the adventure never *shows* why memory was plausible-yet-wrong beyond one line ("The world is small"). Consider one extra clue contrast so the wrong-hypothesis branch teaches hypothesis *revision*, not just "pick the other one." P2 — the current branch is already safe and recoverable.

2. **Heal It — Quilt "indeterminate" could name the next step.** Choosing Quilt correctly refuses to clear the blocker ("needs review"), but doesn't say what review *is* (check the mods' loader support). One clause — e.g. "check each mod's supported loaders" — would turn a dead-end into a teachable next action without exposing catalog internals. P2 — the current message is not wrong, just incomplete.

Neither is a correctness error; both are "could teach one more thing." They do not block SOL-3.

### 21.5 Educational review gate (per LESSON_MAP §9)

- Dependency, loader, Java, recovery, Crash Doctor, and offline claims are **technically correct** and consistent with the authoritative Field Guide. ✔
- All outcomes are simulated; no real authority enters the Lab (enforced structurally by the import boundary and the scenario contract). ✔
- Each takeaway is one short statement linking to an existing Field Guide topic (all 8 referenced topic IDs verified present). ✔
- Terra's perceptual review (TERRA-1..4 for the Lab, TERRA-5 for the live surface) has passed; accessibility/keyboard paths are covered by the linear-view equivalents and the e2e suite. ✔

### 21.6 SOL-3 handoff

~~~text
Agent: Sol
Phase: SOL-3 educational correctness - APPROVED WITH TWO P2 REFINEMENTS
Commit / branch / dirty status: master tracking origin/master; shared uncommitted worktree; no staging, commit, or push performed
Files changed: docs/interactive/ARCHITECTURE_REVIEW_SOL.md only (§21 appended)
Tests run: none (static educational-correctness review of the six scenario reducers, domain models, and guideContent.ts cross-check). Prior gates remain green per IMPLEMENTATION_STATUS.md (unit 190/190, e2e 243/243, cargo 72/72).
How to launch/test: cd desktop; npm run dev; open the "Agora Lab" sidebar tab and play each of the six adventures. Verify each takeaway matches its linked Field Guide topic.
Known failures: none gating. Two P2 refinements for DeepSeek (Fix It wrong-hypothesis revision teaching; Heal It Quilt indeterminate next-step clause). Carried forward non-blocking: Terra P3 wordmark polish; SAFE DEBT list; offline-readiness aggregate query absent (Take It Offline stays simulation-only).
Decisions made: all six adventures teach correct user mental models (dependency simplification, loader compatibility, Java, snapshot/world-backup boundary, Crash Doctor as hypothesis-not-certainty, offline guarantees) and withhold implementation trivia. SOL-3 PASSES.
Decisions explicitly not made: no product-code/test implementation; no capability changes; the two P2 refinements are assigned to DeepSeek, not blocking; no DEEPSEEK-8, SOL-3, Terra, or Luna work; no staging/commit/push
Required next agent: per the coordination plan — TERRA final UX + LUNA final regression, then DEEPSEEK final fixes (including the two P2 refinements above), then SOL-4 final architecture/release review.
Why work is stopping: SOL-3 is a review gate, not implementation; the educational content is approved, and the remaining work is final UX/regression/polish by Terra/Luna/DeepSeek before SOL-4.
~~~

## §22 — SOL escalation review of TERRA-6 / TERRA-6b (2026-08-11)

Terra escalated three findings. Two are BLOCKERs against contracts this review series already
established; one is a contract omission that was never implemented and was mis-scoped as complete.

### 22.1 Verdict summary

| Finding | Classification | Owner |
|---|---|---|
| T6-11 — nullish-equality marks every non-curated mod warning/blocked | **BLOCKER** | DeepSeek |
| T6-3 — `InstanceBench` renders every non-`compatible` loader state unmarked | **BLOCKER** | DeepSeek |
| T6-4 — High Interaction has no settings control and self-disables | **FIX BEFORE RELEASE** (two parts, split below) | DeepSeek |
| T6-12 / T6-13 / T6-14 and the Lab P2/P3 list | FIX BEFORE RELEASE (ordinary) | DeepSeek |

### 22.2 T6-11 — BLOCKER. This is §15 BLOCKER 1 inverted, and the gate series missed it.

`readAdapters/index.ts:97,100` compares `finding.mod_id === mod.registry_id` where both sides are
`string | null`. `null === null` is true, so any finding Agora could not attribute to a specific mod
matches every mod Agora could not attribute to the registry.

This is the same contract as §15 BLOCKER 1 ("partial read failures become false fresh/healthy"),
failing in the opposite direction: **verified-healthy content is rendered as damaged**. I accepted the
fail-closed argument at §20 on the strength of fragment-level tests, and those tests are sound — but every
fixture in this series carries populated `mod_id`/`registry_id`, so the null path was never executed.
Five gates, 243 e2e tests, and a live UX review all passed over it. That is a fixture-coverage failure in
my own gate design, not only a coding error.

The blocker arm (`:97`) is the more serious half: a single unattributed **blocker** would render every
non-curated mod as `blocked`. The observed instance had zero blockers, which is why only the warning arm
fired. Absence of blockers on one instance is not a mitigation.

**Required:** compare identities only when both sides are non-null; attribute findings with neither
`filename` nor `mod_id` to the instance, never to content nodes. **Mandatory regression fixture** with
`mod_id: null` + `registry_id: null` for blocker, warning, and recommendation. I will not clear SOL-4
without that fixture existing.

### 22.3 T6-3 — BLOCKER, and it is a live-surface truthfulness defect, not a Lab polish item

`InstanceBench.tsx:127` renders the "Fits this setup" hint only when `compatibility === 'compatible'`.
Every other value — `incompatible`, `unknown`, `indeterminate` — renders nothing at all. Uncertainty and
incompatibility are both presented as silence, and silence is indistinguishable from a confirmed-good
loader.

`InstanceBench` is a shared visual, and `readAdapters/index.ts:68` sets the live loader's compatibility to
`'unknown'` unconditionally. So on the live surface this is not an edge case — it is the **only** state
that ever renders. Terra confirmed it on a real instance: `Loader   fabric`, unmarked.

This directly violates `SAFETY_BOUNDARIES` / `VISUAL_LANGUAGE`: an unverified state must be visually
distinct from a verified one. The precedent is already set — the TERRA-5 runtime P2 was fixed with an
explicit "Java runtime: not verified" label for exactly this reason. The loader row must match it.

**Required:** render current-loader compatibility for all four states, `unknown`/`indeterminate` visually
distinct from `incompatible`. Do not reuse the bare `Unknown` chip that TERRA-5 already rejected as
ambiguous — use the explicit not-verified wording.

### 22.4 T6-4 — the §18.4 lifecycle does not require destroying the preference. Split approved.

Terra is right that these are two different things, and the current code conflates them.

§18.4 requires that **every bridge leaves High Interaction before Standard work begins**, so the host
unmounts and no stale live state can survive a Standard close/cancel/rejection/failure. That requirement
is about *live session state*. It is fully satisfied by dropping the active view. It says nothing about
the user's persisted presentation preference, and I did not intend it to.

**Ruling — approved architecture:**

- `agora-interaction-preference` remains the user's **intent**: "I prefer High Interaction where it is
  available." Only an explicit user action may write it.
- A separate **session/view state** decides whether the live host is mounted right now. Bridges clear the
  view state, not the preference. §18.4's unmount guarantee is preserved verbatim.
- Re-entering the editor may honour the preference again. This is safe because remount performs a fresh
  read (§18.4) and the render-time instance-identity guard (§19.4) still applies. No stale state can
  survive, because none is retained across the unmount.

This is not a relaxation of §18.4 — the unmount, the fresh read, and the identity guard are all unchanged.
It only stops a safety mechanism from silently rewriting a user setting.

**Second part — the missing control is a contract omission.** `MASTER_ARCHITECTURE.md:139` requires High
Interaction Mode to be "selectable from a clearly named appearance/interaction control", with contextual
offering as the optional addition. Only the optional half shipped. `IMPLEMENTATION_STATUS` recorded
DEEPSEEK-7 as done without it, and none of §15–§21 checked for it, because every review in this series
scoped itself to safety of the *live seams* rather than to §5 coexistence. That is a genuine gap in the
gate coverage and it is fixed by implementing the control, not by amending the contract.

**Required:** add the settings control (safe default `standard` preserved), implement the session/preference
split above, and remove the duplicated escape control (T6-14).

### 22.5 Findings NOT escalated, recorded for completeness

- The mode currently offering only `Stage removal` on content is a faithful consequence of the approved
  seam list (install/update blocked on §19.5's curated-source dependency; enable/disable rejected). It is
  a **product** decision, not a safety defect. Recorded as OPTIONAL for Phase 2; the honest alternative is
  to unblock the curated candidate source rather than to widen the seam list.
- T6-13 (raw jar filenames) must be fixed **without** adding a new live read command. `InstalledMod`
  carries no display name, so the friendly label must be derived presentationally with the filename
  retained as secondary text. A derived label that hides its source would be a new truthfulness problem;
  keeping the filename visible makes the derivation a convenience rather than a claim.

### 22.6 Gate-coverage lesson for SOL-4

Both BLOCKERs share one root cause: **every fixture in this series describes fully-populated, confidently
classified data.** The fail-closed contract was tested for read *failure* but never for read *ambiguity* —
null identities, `unknown` compatibility, unattributed findings. SOL-4 must confirm that the null/unknown
paths now carry explicit tests, not only the error paths.

## §23 — SOL-4 final architecture / release review (2026-08-11)

Scope per the role plan: architecture, performance on large instances, persistence, simulation/live
boundaries, accessibility, Standard-mode regressions, tests/docs, remaining debt. Reviewed after the
TERRA-6 fix batch, TERRA-7 retest, and the LUNA-5 sweep.

### 23.1 Recommendation: **SHIP the Lab. SHIP High Interaction Mode as an opt-in presentation.**

Both §22 BLOCKERs are closed with tests that execute the paths that were previously unreachable. The
gate series is complete: SOL-0 contracts → SOL-1 → SOL-2 (§15–§20) → SOL-3 → §22 → this review.

One condition, and it is not optional: **the null/unknown fixture coverage added in this batch is now
part of the contract.** §22.6 identified that every fixture in this series described fully-populated,
confidently-classified data, and that both BLOCKERs lived in the gap. `readAdapters.test.ts` now covers
`mod_id: null` + `registry_id: null` for blocker, warning, and recommendation, and asserts the
distinction between an unattributed finding and an attributed one. Removing those fixtures re-opens the
defect class silently.

### 23.2 Architecture — sound, and two contract additions approved

- The three-layer separation holds: `domain/` and `visual/` remain pure, `lab/` remains simulation-only,
  `live/` remains the sole app boundary. The boundary checker passes 63 production files and all 24
  negative fixtures, including the alias-laundering and dynamic-import cases from §17.5.
- **`VisualContentNode.fileLabel` (new, approved).** The derived display label needed somewhere honest to
  put the authoritative filename. This is additive, optional, and consumed by `ContentGraph` only. It also
  exposed a latent hazard: the install bridge derived its filename from `node.name`, so a derived label
  would have silently broken removal. `LiveInteractiveHost` now uses `fileLabel ?? name` for the bridge
  payload and `name` for display. That the existing host test caught this is exactly why §17.4's
  action-bearing route tests were worth insisting on.
- **`LabScenario.safeResumeCheckpoint` (new, approved).** This closes SOL-1's SAFE DEBT note about
  resume fidelity properly rather than by convention. Scenarios that branch on unrecorded decisions now
  declare how far resume may restore, and the shell honours it. Mod It needed no clamp once its
  checkpoint-2 scene stopped fabricating the optional; Fix It and Take It Offline clamp to 1.
- **Session view state vs persisted preference (§22.4)** is implemented as ruled. §18.4's guarantee is
  unchanged: bridges still leave High Interaction, the host still unmounts, re-entry still performs a
  fresh read under the §19.4 identity guard. Only the write-through to the user's saved preference is
  gone.

### 23.3 Performance on large instances — verified, not assumed

Previously this was argued from the memoisation in §16.6. It has now been observed: a real 136-mod
instance loads, renders, refreshes, and routes a bridge without stalling. The 12-node spatial cap and
`Show all 136 items` behave as designed, and `contentToVisual` remains O(nodes + relationships).

The bundle is 1,284 kB (360 kB gzip) and still above Vite's warning threshold. Route-level lazy loading
of the Lab and the live host remains the correct fix. **SAFE DEBT** — it does not block release.

### 23.4 Simulation / live boundary — unchanged and enforced

No Lab file imports `@tauri-apps/*`, `@/lib/tauri`, `live/`, or an operation component. No rejected seam
gained a bridge. `liveHighInteractionCapabilities()` still enables only the five Sol-approved seams;
`canProposeInstall`/`canProposeUpdate` remain off pending §19.5's curated candidate source, and
`canProposeEnabled`, `canRequestSnapshotRestore`, `canProposeMemory`, `canReviewOfflineReadiness` remain
off. Terra probed for all of them on real data and found them absent, not merely disabled.

### 23.5 Standard-mode regressions — none

The Standard instance editor, 136-row mod table, install flow, and reviewOnly `HealthDialog` are
unchanged; e2e 243/243 covers them. The one Standard-surface edit in this area (`HealthDialog` reviewOnly
hiding Disable, from §18.5) predates this batch and remains verified.

### 23.6 Accessibility

Operation buttons now carry their target in the accessible name across the shared visuals and the Lab
shell — the defect Terra found was systemic and the fix was applied at the component layer, so it covers
the live surface too. Linear/list equivalents, focus visibility, and reduced-motion neutralisation are
unchanged from TERRA-3. **Owed:** the full viewport / density / high-contrast / 200%-text /
reduced-motion matrix (LUNA-5 note 3). This is the one item I would want run before tagging a release,
but it is a verification gap, not a known defect.

### 23.7 Remaining debt, carried forward

| Item | Class |
|---|---|
| Route-level lazy loading (bundle > 500 kB) | SAFE DEBT |
| Full viewport/density/contrast/reduced-motion matrix | **Owed before tag** |
| No `declined` relationship state (skipped optional reads "Needs review") | OPTIONAL / Phase 2 |
| Offline-readiness aggregate query absent (Take It Offline stays simulation-only) | OPTIONAL |
| Sidebar wordmark truncation at 1366x768 high-readability | OPTIONAL |
| Derived display names are heuristic (filename always retained) | OPTIONAL |

### 23.8 Phase 2 priorities, in order

1. **Unblock install/update.** §19.5's missing curated candidate/target-version source is the single
   thing keeping High Interaction to one content verb — and that verb is the destructive one. Until it
   lands, the mode reads as a bulk-removal tool (§22.5). This is the highest-value next investment.
2. **A real offline-readiness query in core**, so Take It Offline can leave simulation and the
   `NetworkReadinessMap` can render truth instead of `unknown`.
3. **Route-level code splitting** for the Lab and the live host.
4. **Ambiguity fixtures as a standing rule.** Extend §22.6 beyond the read adapters: every layer that
   classifies backend data should have a null/unknown/unattributed fixture, not only a failure fixture.
5. Relationship-state vocabulary (`declined`), then the remaining OPTIONAL polish.

### 23.9 What I got wrong in this series, recorded deliberately

§20 approved the SOL-2 gate on fragment-level fail-closed evidence that was real but incomplete. The
contract I was enforcing — "uncertainty must never read as ready" — had an unexamined converse, and two
defects lived there for five gates: healthy content rendered as damaged (§22.2), and unverified rendered
as verified (§22.3). Both were invisible to every fixture and every mock, and both were found in minutes
against one real instance. The process lesson for future gates is narrow and concrete: **a gate that only
ever sees synthetic data is testing the fixture generator, not the contract.**

### Handoff

~~~text
Agent: Sol
Phase: SOL-4 final architecture / release review
Commit / branch / dirty status: master; uncommitted (63 changed/untracked paths)
Files changed: docs/interactive/ARCHITECTURE_REVIEW_SOL.md (§22, §23)
Tests run (whole batch): frontend unit 211/211 (24 files), e2e 243/243, interactive boundary OK
  (63 files), boundary fixtures 24/24, npm run build green, cargo check -p agora-desktop clean,
  check_architecture.py / check_docs.py / check_tauri_bindings.py all OK.
How to launch/test: cd desktop; npm run dev (the Tauri debug build uses devUrl localhost:5173).
  My Instances -> Edit -> High Interaction view; Settings -> Appearance -> Instance view; Agora Lab.
Known failures: none. Owed before tagging: the full viewport/density/contrast/reduced-motion matrix.
Decisions made: SHIP recommendation for the Lab and for High Interaction Mode as an opt-in
  presentation; approved two contract additions (VisualContentNode.fileLabel,
  LabScenario.safeResumeCheckpoint) and the session-vs-preference split; null/unknown fixture coverage
  is now a standing requirement; Phase 2 priorities ordered with install/update unblocking first.
Decisions explicitly not made: no enabling of install/update or any rejected seam; no relaxation of
  18.4/19.4; no commit, tag, or push.
Required next agent: none. Remaining work is the owed accessibility matrix and Phase 2.
Why work is stopping: the gate sequence is complete.
~~~
