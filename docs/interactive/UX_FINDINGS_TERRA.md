# Terra UX Gate: Vertical Slice

**Phase:** TERRA-1 through TERRA-3

**Initial verdict:** **DO NOT proceed to SOL-1 yet.** The shell is usable and the Lab is unmistakably a simulation, but the three core lessons currently rely too much on prose and two screens teach a false current/proposed or current/restored state.

**TERRA-4 outcome (2026-08-09):** **PASS.** DeepSeek's fix batch resolved every P1 and P2 finding below in a new black-box run. SOL-1 may begin. The deferred P3 app-shell wordmark polish remains non-gating.

## Method

Black-box review in the local Vite preview from a clean Lab entry point. Completed Build It, Mod It, and Undo It as a zero-context player, intentionally chose incompatible/blocked paths where offered, then inspected the minimum source necessary to confirm the observed state contradictions. No real operation was called.

## Zero-context learning result

| Adventure | What the interaction caused me to believe | Result |
| --- | --- | --- |
| Build It | An instance looks like a separate configuration, and a loader must fit the selected Minecraft version. The example bench is probably a different instance. | **Partial fail.** Both panels are named `Instance bench` and marked `Current`; isolation comes mostly from explanatory/completion prose rather than an unmistakable visual boundary. |
| Mod It | A dependency and conflict exist only because the relationship sentences say so. Staging is separate from applying until the end, where the screen then says the applied changes are still proposed and current state is unchanged. | **Fail.** This directly teaches the wrong current-versus-proposed model. |
| Undo It | A return point has a scope, worlds are not always included, the instance must stop, and a serious confirmation is required. After confirming, the page still says that the current state is the bad one. | **Partial fail.** Scope and stopping are clear, but the restore has no visible result beyond text/badges. |

## Findings

### P1 — Applied Mod It state still presents as proposed and unchanged

**Observed:** After `Apply simulated plan`, BetterCaves, Core Lib, and Terrain Overhaul still show `Proposed` / `Proposed: install` / `Proposed: remove`. At the same time the staging dock says `No changes staged yet. Current state is unchanged.` This is a direct contradiction of both the applied feedback and the lesson's core concept.

**Why it matters:** A player learns that applying a reviewed plan neither commits it nor changes current state. This violates the shared current/proposed contract and fails the staged-versus-executed learning goal.

**Likely seam:** `desktop/src/features/interactive/lab/scenarios/modIt.ts:103-137` retains proposed presence after `applied`; Core Lib and Terrain Overhaul never project their applied value into current state. `desktop/src/features/interactive/visual/ChangeStaging.tsx:63-64` consequently renders the false unchanged-state message after proposals clear.

**Required fix:** On apply, make the graph's post-apply state fully current (installed BetterCaves and Core Lib, removed Terrain Overhaul), clear every proposal marker, and replace the empty staging message with an applied outcome/return-point summary for that completed scene. Add a UI test for this exact post-apply screen.

### P1 — The generic serious-confirmation dialog falsely calls mod replacement a restore

**Observed:** Choosing `Replace Terrain Overhaul` opens an alert dialog labelled `Confirm restore` and says worlds are restored only when the selected point includes them. That action is a simulated install-plan replacement, not a restore.

**Why it matters:** It conflates two distinct serious operations and could teach that content replacement is itself a restore. The dialog is likely to be reused for real operations later, so the misconception is safety-relevant.

**Likely seam:** `desktop/src/features/interactive/lab/LabShell.tsx:298-307` hard-codes the restore accessible name and world-restore copy for every dangerous decision.

**Required fix:** Make the dialog's accessible name and consequence copy action-specific. The replacement path should name the content removed/installed and any new recovery point; only an actual restore may describe restoring worlds/scope.

### P1 — Undo It reports a successful restore while retaining the visibly bad current state

**Observed:** After confirmed restore, `Restored` and `Undo point created` appear, but the timeline still reads `Current state — a change made things worse`; the return cards and comparison also show no spatially visible recovery outcome.

**Why it matters:** The player cannot see what recovery returns. The state shown after success contradicts the word `Restored`, so the lesson fails its recovery and scope objective without relying on prose.

**Likely seam:** `desktop/src/features/interactive/lab/scenarios/undoIt.ts:76-91` establishes the bad-current label and `:241-247` completes a restore without replacing it or projecting the selected return point into current state.

**Required fix:** After confirmation, replace the current-state label with the selected return point/outcome, show the restored versus retained scope locally (especially worlds not included), and visually add the pre-restore undo point to the timeline. The pre-restore and post-restore states must be visibly different.

### P1 — Build It does not visually establish instance isolation

**Observed:** The player sees two identical `Instance bench` panels, both tagged `Current`, with no labels distinguishing the new simulated instance from the untouched example. The separate-instance lesson is stated in text rather than demonstrated by the arrangement.

**Why it matters:** The Lab's first main concept—instances do not affect one another—must not depend mainly on explanatory prose.

**Likely seam:** `desktop/src/features/interactive/visual/InstanceBench.tsx:73-86` gives every bench the same name and current phase; the scenario needs role-specific labels or an explicit comparison treatment.

**Required fix:** Present clear, adjacent roles such as `Your new instance` and `Existing example — unchanged`, with one visible effect occurring only on the new instance. Do not represent both as an undifferentiated global `Current` state.

### P1 — Mod It’s declared Diagram view does not visually teach relationships

**Observed:** The selected `Diagram view` is a four-card grid followed by prose relationship rows. Pressing `Snap required: Core Lib` adds a small proposal chip but no visible socket, edge, or causal connection; the required/optional/conflict meanings therefore come from sentences and button labels.

**Why it matters:** This fails the zero-context dependency lesson and the shared rule to show rather than lecture.

**Likely seam:** `desktop/src/features/interactive/visual/ContentGraph.tsx:155-158` only adds textual proposal state, while scenario feedback calls the unrepresented action a `socket` in `modIt.ts:261-269`.

**Required fix:** Either implement a genuine spatial relationship view (nodes + visible required/optional/conflicting links) or rename the mode and remove socket language. Required versus optional and a resolved conflict must be readable from the visual state without its explanatory sentence.

### P2 — Invalid/blocked choices provide only screen-reader feedback after activation

**Observed:** Choosing incompatible Forge in Build It and `Try restore now` while running leave the screen visually almost unchanged. The failure text is announced in the accessibility tree, but it is not visible in the rendered interface; the invalid choices also look like ordinary enabled primary buttons.

**Why it matters:** Wrong predictions are safely blocked, but the visual interface does not make the cause/effect legible. This weakens the interaction grammar for compatible versus invalid targets.

**Required fix:** Preserve the safe learning choice if desired, but show a short inline blocked/rejected state beside the attempted target and distinguish unavailable/invalid action styling before activation. Do not make an incompatible selection appear to be a normal current configuration.

### P3 — High-readability at a 1366x768 laptop viewport clips the launcher wordmark

**Observed:** With the High readability preset, the sidebar brand truncates to `Agora Laun...` at 1366x768. The Lab cards remain readable in compact and spacious density.

**Required fix:** Treat this as an app-shell responsive polish follow-up; it is not a Lab gate blocker.

## Accessibility and interaction checks

| Check | Result |
| --- | --- |
| Simulation boundary | Pass. The persistent Simulation label and explicit no-real-instance statement are clear. |
| Focus visibility | Pass. Native controls expose a high-contrast focus ring. |
| Keyboard route | Controls are semantic buttons and no task is drag-only. The in-app browser input adapter did not dispatch Enter/Space for a focused button, so final native-app/CI activation confirmation remains required. |
| Reduced motion | Pass. `data-motion=reduced` was active; a version-placement state change remained visible without relying on motion. |
| High contrast / light / dark | Pass for Lab content. High-contrast light and high-readability dark kept text, borders, status chips, and actions distinguishable. |
| Large text / density | Pass for Lab content at high readability and both compact/spacious density. See P3 for the separate shell wordmark truncation. |
| Laptop viewport | Pass for the Lab at 1366x768; cards and controls remain readable. |

## Golden interaction standard (for the retest)

- The next safe action is visually discoverable without paragraph instructions.
- A cause and its effect are adjacent in space and clear in time, including under reduced motion.
- Current, proposed, rejected/blocked, and applied states cannot share the same visual treatment.
- Dependencies, conflicts, and snapshot scope are readable from the visual itself; short text may confirm them, not carry the lesson.
- Serious actions state the exact affected content and consequence before confirmation.
- Wrong predictions remain safe and produce visible feedback.
- Reset is obvious, keyboard completion is possible, and the simulation boundary remains persistent.

## TERRA-4 retest — passed

Retested the corrected local Vite preview through Build It, Mod It, and Undo It. No source inspection was needed: every fixed behavior was directly visible in the rendered experience.

| Prior finding | Retest result |
| --- | --- |
| P1 — Build It isolation | **Pass.** `Your new instance` is highlighted and changes from empty to Minecraft 1.20.1, while the adjacent `Existing example` is explicitly `Separate · unchanged` and remains on 1.21/Forge. |
| P1 — Mod It relationship lesson | **Pass.** Diagram view now shows a missing required Core Lib socket, optional Nice Textures, and a visibly changed green `Satisfied` socket once Core Lib is added. This carries the dependency meaning without the old prose relationship list. |
| P1 — replacement confirmation | **Pass.** Replacing Terrain Overhaul opens `Confirm replacement`, accurately names the removed/installed content and the created return point, and no longer calls the action a restore. |
| P1 — applied plan state | **Pass.** After apply, BetterCaves/Core Lib are current installed content, Terrain Overhaul is absent, no `Proposed` marker remains, and an adjacent `Plan applied · Committed` outcome states the result and return point. |
| P1 — restore outcome | **Pass.** The current label changes to the selected automatic return point, a new `Undo restore point` appears in the timeline, and the adjacent restore outcome says instance files returned while worlds were not touched. |
| P2 — incompatible/blocked feedback | **Pass.** Choosing Forge and trying a running-instance restore now render red inline feedback beside the attempted action as well as announcing it. |

The Golden interaction standard above is now met for this vertical slice: next steps are discoverable; current/proposed/rejected/applied states differ visibly; dependency and recovery scope have local visual evidence; serious actions name their actual consequence; and wrong predictions stay safe with visible feedback.

## Handoff

```text
Agent: Terra
Phase: TERRA-4 retest and Golden interaction-standard confirmation
Commit / branch / dirty status: master; uncommitted. Existing shared interactive work remains uncommitted.
Files changed:
  docs/interactive/UX_FINDINGS_TERRA.md (Terra retest result appended)
Tests run:
  Black-box local Vite retest of all three vertical-slice adventures, including the fixed instance-isolation, dependency socket, action-specific confirmation, post-apply, post-restore, and inline blocked-feedback paths. DeepSeek separately reports build, 71 unit tests, and 241 Playwright tests passing.
How to launch/test:
  cd desktop; npm run dev; open Agora Lab. The retest paths are documented in the TERRA-4 table above.
Known failures:
  No remaining Terra P0/P1/P2 gate finding. The P3 high-readability sidebar wordmark truncation remains a non-gating app-shell follow-up.
Decisions made:
  TERRA-4 clears the Lab vertical slice for SOL-1. The findings assess user-facing meaning and safety presentation only; no shared visual-domain contract was changed.
Decisions explicitly not made:
  No code fixes, architecture changes, live adapters, High Interaction Mode work, P3 polish, or product-scope expansion.
Required next agent:
  Sol for SOL-1 architecture scaling gate. Routine visual regression after future implementation batches belongs to Luna; Terra resumes at TERRA-5 for live High Interaction review.
Why work is stopping:
  TERRA-4 is complete and the coordination sequence now requires Sol's architecture gate before further Lab implementation.
```
