# Interactive Safety Boundaries

Status: SOL-0 safety contract

Applies to: Agora Lab, High Interaction Mode, shared visuals, adapters, controllers, tests, and future lesson content

These boundaries are release requirements, not suggestions. A playful presentation must not weaken Agora’s existing secure defaults, backend authority, reversibility, or accessibility.

## 1. Authority map

| Layer | May read live data | May create a local proposal | May request an existing review flow | May execute backend work |
|---|---:|---:|---:|---:|
| Shared visual component | No service access; receives mapped data | Yes, through `VisualIntent` | No direct access; emits intent only | Never |
| Lab lesson engine/simulation adapter | Never | Yes, simulated only | May navigate out of Lab after explicit exit | Never |
| Live read adapter | Yes, read-only commands only | No | No | Never mutating work |
| Live intent controller | Yes | Coordinates proposal state | Yes | Not directly; delegates to an existing authoritative bridge |
| Existing Standard controller/review surface | Yes | Yes | Owns review/confirmation | May call existing Tauri command after review |
| Tauri/core | Yes | Resolves authoritative plans | Enforces policy/validation | Sole authority for mutation, process, rollback, and recovery |

If an implementation cannot clearly identify which row owns an action, it must stop at read-only visualization.

## 2. Lab isolation

Lab must be unable—not merely instructed not—to mutate real state.

### Required structural controls

- `lab/`, `visual/`, and `domain/` cannot import `@tauri-apps/*`, `lib/tauri`, `live/`, operation components, process controllers, or settings stores.
- Lab’s root receives no live instance ID, backend client, mutation callback, filesystem path, process handle, account state, or network-policy setter.
- Scenario fixtures use namespaced simulated IDs and authored outcomes.
- The lesson reducer is pure and deterministic.
- Lab progress stores only non-sensitive checkpoint metadata.
- Leaving Lab is an explicit navigation event. Simulation state is discarded or retained only as Lab progress; it is never translated into a live plan.

DeepSeek must enforce the import boundary with a fast automated check and scenario contract tests. A runtime `if (isLab)` guard is not sufficient.

### Lab labels

The Lab shell displays `Simulation` persistently. Completion language says “You completed the practice” rather than “Your instance is fixed/ready.” Links to real Agora end the simulation before navigation.

## 3. Shared visual isolation

A shared visual is a controlled component over presentation models. It may emit a `VisualIntent`; it may not:

- import or call Tauri;
- call an existing mutation wrapper;
- construct backend DTOs or command names;
- parse backend prose to select an operation;
- retain private plan/scan tokens;
- optimistically mark live state committed;
- create its own process, install, health, or recovery state machine.

This allows Lab and live views to share the grammar without sharing authority.

## 4. Live intent gate

Every live intent passes all of these gates in order:

1. **Source gate:** reject an intent whose scene is simulation, missing, or mixed-source.
2. **Capability gate:** the host explicitly enables only actions implemented through an approved bridge.
3. **Freshness gate:** compare the intent’s view revision and re-read relevant state.
4. **Availability gate:** honor process state, instance locks, recovery readiness, and active operations.
5. **Review gate:** open the existing Standard review/dialog/controller and show refreshed current versus proposed state.
6. **Backend gate:** core revalidates the plan, token, health state, policy, and resource locks.
7. **Outcome gate:** render applying/progress from authoritative events; no local success.
8. **Refresh gate:** after success, failure, cancellation, or rollback, re-read before showing current state.

Only the existing authority path may cross gate 6. The visual intent controller is routing and coordination, not a second business-logic layer.

## 5. Current and proposed state

Current state is the last successful authoritative read. Proposed state is local and reversible until the existing review surface accepts it.

- A proposal cannot overwrite or visually replace current state before commit.
- Proposed removal/disable remains named and visible.
- Applying is distinct from committed.
- Navigation or animation completion does not commit work.
- A rejected, stale, cancelled, or failed proposal never leaves a success mark.
- A rolled-back operation reports rollback and refreshes; it does not claim the original change succeeded.
- Lab current/proposed state is always enclosed by the persistent Simulation context.

## 6. Stale-state behavior

Live scenes are snapshots in time. A visual intent carries a local view revision and stable IDs only. Before any review bridge opens, the live controller re-reads the relevant entities.

If state changed:

- refresh the scene;
- explain the material change in player language;
- discard an invalid proposal;
- retain selection or reconstruct an equivalent proposal only when doing so cannot broaden its effect;
- require fresh review for any revised dependencies, conflicts, target version, loader choice, snapshot scope, memory value, or process state.

Install plan fingerprints, health scan tokens, and other backend authority values remain private to existing flows and are validated again by core. A stale-backend rejection is normal concurrency behavior, not a reason to retry a mutation automatically.

## 7. Destructive and consequential action matrix

| Action | Visual may do | Mandatory live bridge | Confirmation/recovery requirement | SOL-2 state |
|---|---|---|---|---|
| Select, filter, inspect, compare | Perform locally on mapped data | None | No mutation; preserve focus and source label. | Read-only eligible |
| Install or update content | Stage intent and show a non-authoritative preview | Existing `InstallFlow` | Backend-resolved dependency/conflict/file/snapshot review; cancellation and rollback retained. | Approval required before enabling gesture-to-live bridge |
| Remove content | Stage and show affected names/counts | Existing `InstallFlow` | Explicit destructive review, dependency effects, recovery point, rollback retained. | Approval required |
| Disable content | Stage intent only | Existing dependency plan and prompt | Review dependents; one direct graph toggle is forbidden. | Approval required |
| Change loader | Select candidate for review | Existing loader plan/chooser/change flow | Proven vs indeterminate shown; affected content reviewed; post-change health refresh. | Approval required |
| Launch with health findings | Open health review | Canonical process controller and `HealthDialog` | Blockers and override behavior unchanged; warnings/recommendations keep existing semantics. | Approval required for live visual trigger |
| Crash experiment | Open Crash Doctor | Existing `CrashInvestigator` | Read-only evidence first; recovery snapshot before mutation; dependency review; restore on failed/abandoned experiment. | Approval required |
| Restore snapshot | Select/compare only | Existing restore command behind a serious review/confirmation bridge | Scope/world boundary, active-process block, pre-restore undo point, backend transactional restore. | Explicitly blocked until SOL-2 approves the bridge |
| Change memory/runtime | Stage current-to-proposed choice | Existing settings update flow | Show recommendation/headroom; validate; save only after explicit review. | Approval required |
| Stop/kill process | Request through app-level process controller | Existing stop/kill path | Preserve graceful-stop and kill distinctions, delegated rules, and stale-process handling. | Approval required |
| Change privacy/network policy | Navigate to existing Settings | Existing Privacy settings | Preserve backend enforcement and consequential policy wording. | No direct visual manipulation in initial scope |

“Approval required” means the architecture identifies a possible route but does not authorize implementation. SOL-2 must answer the eight operation-safety questions in the role plan for each action.

## 8. Existing recovery behavior is indivisible

An interactive bridge must retain the whole existing operation contract, not only the happy-path command:

- install plan resolution, user choices, fingerprint validation, staging, snapshot, apply, health, cancellation, rollback, and result;
- health scan identity, blocker/warning/recommendation semantics, and explicit override rules;
- loader compatibility evidence, plan, mutation, and fresh post-change health;
- snapshot readiness, process/launch exclusion, pre-restore undo point, validation, swap, and rollback;
- Crash Doctor’s deferred recovery snapshot, dependency-aware experiment, canonical relaunch, correlated outcome, and restoration on failure/abandonment;
- app-level launch/process state across navigation;
- backend network policy and secure delegated-launch default.

If a visual route cannot preserve the complete contract, it must navigate to Standard Mode before the operation begins.

## 9. Locks, cancellation, and rejection

- Backend operation locks and launch reservations are final. Visual availability is advisory and may become stale.
- A lock/busy rejection does not trigger an automatic retry or clear a proposal without explanation.
- Cancellation is offered only where the existing operation supports it and is not displayed as complete until acknowledged.
- Leaving the interactive view does not imply cancellation; the canonical app-level operation state continues as it does in Standard Mode.
- A backend validation or policy rejection is shown as a refreshed current state plus actionable explanation, never as a broken animation.
- Duplicate gestures are coalesced/disabled while the corresponding intent is in review or applying, but accessible focus and status remain available.

## 10. Animation and causality safety

Animation observes state; it does not define state.

- A proposal animation may begin after the reducer accepts a local proposal.
- Live apply animation may reflect backend progress events.
- Success animation begins only after backend success and fresh read.
- Failure, cancellation, staleness, and rollback have explicit non-success states.
- Reduced motion follows the same state transitions without waiting for animation callbacks.
- No destructive action is confirmed by dropping, shaking, swiping, holding for a duration, or watching an animation alone.

## 11. Offline-readiness boundary

The current code does not provide a single authoritative read-only aggregate offline-readiness result. Therefore:

- Take It Offline is simulation-only in Lab initially.
- High Interaction Mode cannot claim live readiness from registry state, directory inspection, or partial cache facts.
- It cannot use a launch, download, repair, Java install, sign-in, or other mutation as a “check.”
- A future command must be core-owned, read-only, policy-aware, instance- and launch-mode-specific, and explicit about unknown categories.
- The presentation hides cache paths, receipt details, and hashes while preserving player-visible missing/blocked/unknown reasons.

Adding this query is an implementation/design decision for a later phase, not part of SOL-0.

## 12. Privacy and data minimization

- Lab progress contains no live player data.
- Live adapters request only the data needed for the visible scene and discard raw DTOs when practical.
- Crash evidence defaults to bounded summaries; full logs/paths are not placed on a playful board by default.
- Account identifiers, tokens, credentials, private keys, and authorization headers never enter presentation models, telemetry, fixtures, screenshots, or Lab data.
- Network access remains controlled by existing backend policy. A visual cannot bypass lockdown or infer permission from UI state.
- Optional AI remains optional and follows the existing Crash Doctor privacy boundary; Lab does not silently send scenario or live data externally.
- Persisted interaction preference and Lab progress are versioned and local.

## 13. Accessibility is a safety boundary

An inaccessible equivalent is not a secondary enhancement; it is required before the spatial action ships.

- Every gesture maps to the same `VisualIntent` as a named keyboard/list action.
- Focus does not enter decorative shapes and returns predictably after dialogs and refreshes.
- Current/proposed, blocker/warning/recommendation, known/unknown, and compatible/indeterminate do not rely on color, position, or motion alone.
- Serious consequences and scope boundaries are exposed in text to screen readers.
- Reduced motion has no timing-dependent lesson or confirmation.
- High contrast retains outlines/connectors/focus; 200% text reflows to the linear representation.
- Disabled controls do not hide the reason or strand focus.

A missing keyboard or screen-reader operation path blocks the corresponding pointer gesture from release.

## 14. Test and review gates

Before the vertical slice can pass to Terra/SOL-1:

- automated import checks prove Lab/shared visual code has no live authority dependencies;
- scenario tests prove Lab intents cannot reach a backend mock;
- controlled-component tests prove visuals only emit declared intents;
- adapter tests prove giant DTO/private authority fields are omitted;
- live-controller tests prove review precedes any mutation and staleness causes refresh/review;
- reduced-motion tests do not rely on animation completion;
- keyboard/screen-reader-oriented tests exercise every implemented gesture;
- existing navigation, appearance, install, health, snapshot, and Crash Doctor regressions remain green for touched seams.

Before any live consequential action ships, SOL-2 approval and targeted native/E2E validation are required.

## 15. Stop/escalation conditions

Implementation must stop and return to Sol when any of these occurs:

- a shared model needs raw backend authority data to function;
- Lab appears to require a live read or mutation;
- an interaction would bypass or duplicate an existing plan/review/recovery flow;
- a stale proposal would need automatic mutation retry;
- a visual cannot distinguish proposed from current without animation;
- offline readiness would require guessing;
- a destructive action lacks a serious accessible confirmation;
- a new dependency or relationship meaning changes the shared visual-domain contract.

Subjective learnability and perceptual questions route to Terra. Ordinary implementation bugs route to DeepSeek. Visual smoke/regression work routes to Luna. Architecture, safety, and disputed domain behavior remain with Sol.
