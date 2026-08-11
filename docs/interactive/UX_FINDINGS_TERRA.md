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

## TERRA-5 — live High Interaction deep review (2026-08-10)

**Phase:** TERRA-5. **Scope:** the real-data High Interaction surface (`LiveInteractiveHost` + `LiveSceneView`) reached from the Standard Instance Editor via the `High Interaction view` toggle, driven black-box against the full read-command mock (the same shape as `e2e/high-interaction.spec.ts`). I exercised the approved seams as a user and probed for the rejected seams. No source change was made.

**Verdict: PASS — no P0/P1 finding.** Every Sol-approved seam that is reachable preserves Agora's authority, current/proposed distinction, and destructive-action seriousness. Every rejected/blocked seam is genuinely absent from the rendered surface, not merely disabled-looking. One P2 and one P3 polish note below; neither gates SOL-3.

### Approved-seam results (priority order from the role plan)

| Seam | Result | Evidence |
| --- | --- | --- |
| Health / loader repair review | **Pass.** `Review health` leaves High Interaction and opens the Standard reviewOnly `HealthDialog`. It contains **zero** Disable/Fix/Repair buttons — the rejected direct-disable seam is not exposed (§18.5 holds). The warning is inspectable read-only with its `Show on next launch` toggle. | `disableButtons: 0, fixRepairButtons: 0`; `stillInHighInteraction: 0` after click. |
| Dependency-aware remove | **Pass.** `Stage removal` leaves High Interaction, opens the canonical `InstallFlow` (`Review Instance Changes`), and issues a backend `resolve_install_plan` — the playful gesture creates intent only; the backend re-resolves the authoritative plan. The review names the consequence (`-1 to remove`, `Snapshot: Before removing example.jar`) and gates mutation behind an explicit `Remove Safely`. **Cancel issues no `apply_install_plan`** and returns cleanly. | install calls = `["resolve_install_plan"]` only; dialog shows snapshot + `Remove Safely`/`Cancel`. |
| Snapshot preview / restore | **Pass (boundary holds).** No snapshot data in this mock → `RecoveryTimeline` does not render, and there are **0 restore / 0 compare buttons**. `canRequestSnapshotRestore` is OFF; no restore gesture exists. | `restoreButtons: 0, compareButtons: 0`. |
| Crash Doctor evidence | **Pass (boundary holds).** No crash evidence → `CrashEvidenceBoard` does not render; **0** Crash Doctor buttons. The approved seam is navigation-only (no experiment). | `crashDoctorButtons: 0`. |
| Memory allocation | **Pass (rejected seam absent).** `RuntimeWorkbench` shows current (`2 GB configured`) vs recommended (`4 GB`) but offers **no** proposal/apply action — `canProposeMemory` is OFF. Current vs recommended is visually distinct; nothing implies a committed change. | `memoryButtons: 0`. |
| Install / update | **Pass (blocked seam absent).** `canProposeInstall`/`canProposeUpdate` OFF pending the curated source (§19.5); no install/update gesture is offered on content nodes. | only `Stage removal` present on the mod row. |

### Cross-cutting safety properties verified live

- **Current vs proposed distinct:** the live scene is a read-only current view; the only "proposal" is the staged removal, which is handed to InstallFlow rather than committed locally. No local success mark appears before backend success.
- **Terminal lifecycle (§18.4):** every bridge leaves High Interaction before Standard work; re-entering High Interaction remounts and performs a **fresh** `get_instance_detail` read (no stale state survives a Standard close/cancel).
- **Refresh (§16.2):** `Refresh` triggers a real re-read and keeps the scene visible throughout (never a bare loading screen). Measured: `get_instance_detail` count 0→1, scene visible during and after.
- **Escape hatch:** `Use Standard view` is always present and returns cleanly to the Standard editor.
- **Keyboard path:** the interactive actions are semantic `<button>`s, focusable with a visible focus ring (1.6px + box-shadow ring), and operable without drag. `ContentGraph` offers a `List view` (searchable linear equivalent) alongside `Diagram view`, satisfying the no-drag-only / linear-reflow requirement.
- **Honest copy:** the header reads "Reviews open the Standard surface; content changes use the reviewed InstallFlow" — it no longer overclaims "read-only" now that remove is enabled.

### Findings

**P2 — Runtime "System Java / Unknown" triple-state is ambiguous next to a real recommendation.** The Runtime workbench shows `Runtime: System Java` with a separate `Unknown` chip immediately beside it, then a confident `recommended 4 GB`. A zero-context user cannot tell whether "Unknown" refers to the Java runtime health, the source, or a failed read — and the confident memory recommendation sits directly under an "Unknown" marker, which slightly undercuts the fail-closed story. **Suggested fix (DeepSeek, non-gating):** label the unknown explicitly (e.g. "Java runtime: not verified") and keep it visually tied to the Runtime row only, so the memory recommendation's confidence is not misread as also unverified. Confirm against `readAdapters` whether this "Unknown" is a genuine indeterminate Java state or a degraded fragment; if it is a failed read it should use the unavailable-note treatment, not an inline chip.

**P3 — Horizontal crowding at the moment of first paint on narrow laptop widths.** On first render at a collapsed-sidebar width the `Refresh` button and the `Need attention` stat sat at the clipped right edge until the layout settled; a document-level check at 1024px shows no persistent overflow (`scrollWidth === clientWidth`), consistent with the L-001 fix. This is a transient first-paint crowding, not a regression. **Suggested follow-up (non-gating):** Luna may fold a first-paint overflow assertion into the existing narrow-viewport regression matrix.

### Accessibility as interaction (live surface)

| Check | Result |
| --- | --- |
| Keyboard-only route to each enabled gesture | Pass — semantic buttons, visible focus, no drag-only task. |
| Linear/list equivalent | Pass — `List view` provides searchable linear content access. |
| Rejected-seam controls hidden, not teased | Pass — rejected actions are absent (not disabled-with-tooltip), so they cannot strand focus or imply availability. |
| Serious consequence in text | Pass — InstallFlow names the removal + snapshot before `Remove Safely`. |
| Escape / Standard parity | Pass — persistent `Use Standard view`. |
| Reduced motion | Not re-triggered here (no animation-dependent transition in these flows); the underlying `data-motion=reduced` guarantee from TERRA-3 is unchanged. |

### Learning transfer (Lab → live)

The visual grammar learned in the Lab transfers: the `Health lens` severity hierarchy (blocker/warning/recommendation), the `Content graph` "what relies on what", and the current-vs-proposed discipline all read the same in the live surface. The one gap is the Runtime workbench's `Unknown` chip (P2 above), which is the only place the live surface's state language is less legible than its Lab counterpart (`Heal It` teaches a clearer proven/indeterminate distinction).

## TERRA-6 — final exploratory UX pass (2026-08-11)

**Phase:** TERRA-6 (final Terra UX gate). **Scope:** the six Lab adventures played zero-context from a
clean profile, plus the discoverability and lifecycle of High Interaction Mode as a *mode*.

**Verdict: DO NOT proceed to SOL-4 yet — 4 P1 findings.** The Lab is close. Undo It, Fix It, and Heal It's
loader rail are the best teaching in the product, and every TERRA-3/TERRA-4 fix I retested still holds.
But two of Mod It's three named learning objectives are broken on the primary path, Build It contradicts
its own stated rule, and High Interaction Mode was never given the settings control its own architecture
contract requires. None of these is dangerous; all four teach or expose something wrong.

### Method and scope limits

Black-box in the local Vite preview at `localhost:5173`, clean profile (`localStorage` held only
`agora-ui-preferences` — no Lab progress, no interaction preference). Played all six adventures as a
new user, deliberately choosing invalid/wrong paths. Source was inspected only *after* observing each
problem, to locate the seam.

Two honest limits on this pass:

- **No screenshots.** The Browser pane was not displayed, so the page never composited frames. Everything
  below is from the accessibility tree, rendered text, and computed DOM — which is the right lens for
  meaning and a11y, but means I did **not** re-verify colour, contrast, spacing, or density. Those remain
  TERRA-3's last passing result plus Luna's regression matrix.
- **The live High Interaction surface was not re-exercised.** The browser preview has no Tauri backend, so
  there are no real instances. TERRA-5 (2026-08-10) covered that surface against the full read mock and
  passed; nothing below changes that verdict except finding **T6-3**, which reaches a shared visual the
  live surface also renders.
- Enter/Space did not activate focused buttons through this input adapter — the same harness limitation
  recorded in TERRA-3. All controls are semantic `<button>` elements, so this is **not** reported as a
  keyboard failure; native/CI confirmation still belongs to the e2e suite.

### Zero-context learning result

| Adventure | What the interaction caused me to believe | Result |
| --- | --- | --- |
| Build It | An instance is a separate named configuration with its own version, loader, and content; my bench changed while the neighbour never did. A loader has to fit the version. | **Partial fail.** Isolation is excellent. But an incompatible loader was accepted as ordinary current state (**T6-3**). |
| Mod It | A change starts as a proposal; a required dependency is a socket that visibly fills; applying commits everything at once. | **Fail on conflict.** Dependency and staged-vs-applied are excellent. "Conflict" — a named objective — is prose-only on the primary path (**T6-1**), and resuming silently changed a decision I made (**T6-2**). |
| Heal It | Blockers stop launch, warnings need review, recommendations never block. Proven-compatible ≠ needs-review. A warning I keep stays. | **Pass**, with a false closing claim (**T6-6**). The loader rail's `satisfied · uncertain · failed` counts are the single best piece of teaching in the Lab. |
| Fix It | Evidence yields hypotheses of differing strength; you change one thing with a recovery point first; one launch supports, it does not prove. | **Pass**, strongest epistemics in the product. The hypothesis under test is never marked, and the first experiment's confirmation can describe the wrong experiment (**T6-5**). |
| Undo It | Recovery has a scope; some return points exclude worlds; the instance must stop; a pre-restore undo point is created. | **Pass — exemplary.** Every TERRA-3 P1 fix holds and the post-restore state is visibly different. |
| Take It Offline | Ready ≠ cached; Unknown stays Unknown; direct launch needs a sign-in that can't be verified offline. | **Pass.** Honest fail-closed framing, and the completion message correctly reflects the branch I actually took. |

---

### T6-1 (P1) — Mod It: the conflict lesson is invisible on the primary path

**Observed.** Playing straight through to checkpoint 2, whose goal text reads *"A conflict surfaced.
Choose how to resolve it before applying."*, the word "conflict" appears **exactly once on the entire
screen**, inside one staging-dock sentence: `Adds BetterCaves, adds Core Lib — conflict with Terrain
Overhaul unresolved.` The Terrain Overhaul node renders as
`<div><button><span>Terrain Overhaul</span><span>Mod</span></button></div>` — no conflict socket, no
`Blocker` chip, no conflicts count, no edge, no marker of any kind on either node.

Exiting and choosing **Resume** returns to the *same step* and renders a completely different screen: an
amber `role="region" aria-label="Conflicts"` panel reading *"BetterCaves / Conflicts with / Terrain
Overhaul / Blocking / BetterCaves and Terrain Overhaul both change world generation."*, a
`Conflicts with Terrain Overhaul · Blocking` socket on BetterCaves, and `Blocker` + `conflicts 1` on both
nodes. Reproduced twice, including once from cleared progress.

**Why it matters.** Conflict is one of the three concepts TERRA-1 requires Mod It to teach, and the
failure rule is explicit: a lesson fails if the correct answer comes mainly from reading explanatory
prose. Here it comes *entirely* from one clause. The player is asked to resolve something the interface
never shows them. Worse, the same step teaches two different lessons depending on whether you took a
break — the resumed version is the good one.

**Seam.** `desktop/src/features/interactive/lab/scenarios/modIt.ts` — `reduce()` (lines 254–347) never
sets `conflictVisible`; only `sceneAt()` does, at lines 170–178. `relationships()` (lines 76–79)
early-returns without the conflict relationship whenever `conflictVisible` is false, so the node
summaries, health marks, and the `ContentGraph` conflict region all stay empty.

**Note for whoever fixes this:** `ContentGraph` already implements the whole thing correctly
(`ContentGraph.tsx:154`, `252–253`, `314–318`, `389`). Nothing needs designing — the scenario just never
turns it on. This should be a very small fix.

**Required fix.** Set `conflictVisible` in `reduce()` at the transition into the conflict checkpoint, so
the primary path renders what the resume path already does. Add a UI test asserting the Conflicts region
and the node `conflicts` count are present at that checkpoint when reached by play, not only by restore.

### T6-2 (P1) — Mod It: resuming silently reverses a decision the player made

**Observed.** I chose **Skip optional textures**. Resuming at the same checkpoint rendered
`Recommends Nice Textures · Satisfied`, gave Nice Textures a `Proposed: install` marker, and grew the plan
summary to `Adds BetterCaves, adds Core Lib and Nice Textures.` The subsequent replacement confirmation
still described only *"BetterCaves (plus Core Lib)"*, so the confirmation now under-describes the plan the
dock is showing.

**Why it matters.** Mod It's core contract is that a proposal is reviewed before it becomes real. A
proposal that silently grows an item the player explicitly declined teaches the opposite, and it does so
in the one adventure whose job is to build trust in staging. Sol already anticipated exactly this in
SOL-1 (`ARCHITECTURE_REVIEW_SOL.md:237`: complex branches "should resume at a clearly documented safe
checkpoint rather than silently reversing a meaningful choice") and accepted it as SAFE DEBT. It is no
longer only debt — it is corrupting a lesson.

**Seam.** `modIt.ts` `sceneAt()` lines 170–174 set `optionalAdded = true` unconditionally for
`checkpoint >= 2`, discarding `optionalSkipped`.

**Required fix.** Persist the branch-relevant decisions in progress, or resume at the last checkpoint
whose scene can be reconstructed without inventing a choice. Whichever route, a resumed scene must never
show a decision the player did not make. Worth checking Fix It and Take It Offline for the same class of
loss before closing.

### T6-3 (P1) — An incompatible loader renders as ordinary current state (shared visual)

**Observed.** In Build It, choosing Forge for a 1.20.1 instance writes Forge into the bench as the current
loader. The chip markup is *identical* to the valid neighbour's:
`rounded-md border border-border bg-card px-2 py-0.5 text-sm font-medium text-foreground`. The only
difference anywhere is that the valid one carries a `text-muted-foreground` caption "Fits this setup" and
the invalid one carries **nothing**. `Need attention` ticks 0→1, but that stat sits far from the loader
row. One step later the Lab states: *"An incompatible tile cannot become current state."*

Heal It handles the identical situation correctly — choosing the unproven Quilt leaves Forge as `Current`
and refuses the change with adjacent inline feedback.

**Why it matters.** Three problems stack. The invalid state is marked only by an *absence*; the positive
signal is styled as de-emphasised secondary text while the negative gets no treatment at all, inverting
salience; and the Lab contradicts a rule it states out loud. This also reopens the half of TERRA-3's P2
that TERRA-4 closed on the inline-feedback evidence alone — the requirement was *"Do not make an
incompatible selection appear to be a normal current configuration."*

**Escalate to Sol — this is not only a Lab bug, and the live case is arguably worse.** The seam is
`desktop/src/features/interactive/visual/InstanceBench.tsx:127`:
`hint={loaderCurrent.compatibility === 'compatible' ? 'Fits this setup' : undefined}`. Every non-compatible
value — `incompatible`, `unknown`, `indeterminate` — renders no mark at all. A `CompatibilityChip` already
exists and is applied to the *proposed* loader two lines below (`:133`) but never to the current one.

`InstanceBench` is a **shared visual the live High Interaction surface also renders**
(`live/LiveSceneView.tsx:16,71`). On the live path the adapter sets the current loader's compatibility to
`'unknown'` unconditionally — `live/readAdapters/index.ts:68`,
`compatibility: 'unknown', // loader compatibility requires a health/compat read`. Combined with `:127`,
that means **every real instance's current loader renders as a bare unmarked chip**, so a loader Agora has
not verified is visually indistinguishable from one it has confirmed good. That is the fail-closed
presentation rule inverted: uncertainty is rendered as silence.

This is the exact sibling of the TERRA-5 P2 I raised on the Runtime row, which came from the same
`compatibility: 'unknown'` pattern at `readAdapters/index.ts:283` and *was* fixed with an explicit
"Java runtime: not verified" label. The loader row has the same defect and was not fixed. TERRA-5 did not
catch it because I was reading the Runtime workbench, not the bench's loader row.

**Required fix.** Render the current loader's compatibility explicitly for every state, not only
`compatible`, keeping `unknown`/`indeterminate` visually distinct from `incompatible` — mirroring the
treatment already applied to the runtime row. Separately, `buildIt.ts:222–231` should decide whether an
invalid loader may enter the bench at all; Heal It's refuse-and-explain behaviour is the better model and
would make the two adventures consistent.

### T6-4 (P1) — High Interaction Mode has no setting, and it turns itself off

This is the concern that prompted the review, and it holds up. It is a contract violation, not a
preference call.

`MASTER_ARCHITECTURE.md:139` (SOL-0, §5.2): *"High Interaction Mode is a reversible presentation
preference… **It should be selectable from a clearly named appearance/interaction control** and **may
also** be offered contextually where a supported live visual exists."*

Only the optional half was built. Verified:

- `AppearanceSettings.tsx` and `Settings.tsx` — **zero** references to High Interaction, Lab, or the
  interaction preference.
- `guideContent.ts` (36 guide pages) — **zero** references to High Interaction Mode.
- The only entry point in the entire app is the `High Interaction view` button at
  `InstanceEditor.tsx:1266–1272`.

Three consequences follow, and the second and third are the substantive ones:

1. **Unreachable on a fresh profile.** With no instances there is no Instance Editor, so the mode cannot
   be discovered at all. A user who finishes all six Lab adventures — which exist to teach this visual
   language — is never told where to use it. Every completion screen links to My Instances, Settings,
   Browse, and the Field Guide; none mentions High Interaction Mode.

2. **It is a global preference driven by a per-instance control.** `agora-interaction-preference` is a
   single global key, read at mount (`InstanceEditor.tsx:1120`). Turning it on while editing instance A
   silently changes how instance B opens, with no global affordance that says so.

3. **The mode disables itself permanently, on purpose, as a side effect of using it.** Every approved
   bridge calls `setHighInteractionPref(false)` (`InstanceEditor.tsx:1153, 1160, 1164, 1176, 1192–1227`),
   and that setter writes through to disk (`:1121–1124` → `savePreference('standard')`). Nothing ever
   restores it. So doing any review — health, loader, snapshot, Crash Doctor, remove, i.e. the mode's
   entire purpose — permanently reverts the user's saved preference to Standard. Someone who wants to work
   in High Interaction must re-enable it, per instance, after every single action.

   Point 3 is worth separating from the safety rule it came from. Sol's §18.4 requires that every bridge
   **leaves High Interaction before Standard work begins** so the host unmounts and no stale live state
   survives. That requirement is about *session view state*. It does not require destroying the user's
   *persisted preference* — but the current code conflates the two, because one setter does both.

**Required fix (routes to DeepSeek; the split in 3 is Sol's call).**
- Add the missing control to the appearance/interaction settings, named as a mode, with the safe
  `standard` default preserved.
- Separate session view state from the persisted preference: bridges should drop the session view (keeping
  §18.4's unmount guarantee intact) without rewriting `agora-interaction-preference`.
- Give the Lab and/or the Field Guide one sentence pointing at where the learned grammar is used. Right now
  the Lab teaches a language the product never tells the user it speaks.

---

### P2 findings

**T6-5 — Fix It: the hypothesis under test is never marked, and the first confirmation can describe the
wrong experiment.** After selecting "not enough memory", all three hypotheses remain
`aria-pressed="false"` and labelled `Candidate`; nothing indicates which one is being tested. The
confirmation then reads *"A simulated recovery point is created first, then **one mod is disabled** and
the game is launched once."* — describing a mod experiment for a *memory* hypothesis, and naming neither
the hypothesis nor the mod. The second experiment's confirmation gets this right (*"This disables the
startup mod and launches once"*), which is the standard the first should meet. Cross-check against the
report's own golden standard: serious actions must state the exact affected content and consequence.
After cancelling, the `Test:` buttons are gone, so the only route to a different hypothesis is Reset —
which is the mechanism behind SOL-3's still-open P2 #1 about hypothesis revision.

**T6-6 — Heal It claims "the check is green" while a warning is on screen.** I deliberately kept Java 8
and was correctly warned *"You can proceed, but launch may fail."* The completion message then states
*"…warnings were reviewed, and the check is green."* while the panel still shows `1 warning` and an
`Incompatible` Java row, and the staged manual-memory change is left dangling as `Proposed`, never applied
or withdrawn. The static `completionMessage` does not reflect the branch taken. Take It Offline shows the
right pattern — its completion accurately said I *"did not turn Unknown into Ready"* for the path I chose.

**T6-7 — Mod It has a dead control.** Once Core Lib is proposed, its node still offers an enabled
`Stage install`. Clicking it leaves the rendered page **byte-for-byte identical** with no new
announcement (verified across separate round-trips; the live regions still held the previous message).
BetterCaves handles the identical case correctly, replacing the button with the persistent reason
*"Already proposed — review it in the staging dock."* A silent no-op is worse than a blocked message and
breaks the "wrong predictions produce visible feedback" standard.

**T6-8 — The "discover" step of three adventures reveals nothing.** Heal It renders all blockers,
warnings, and recommendations *before* `Run validation check`; Fix It renders all four clues and three
hypotheses before `Read the clues`; Take It Offline renders every readiness row before `Inspect
readiness`. The effect precedes the cause, so the first action of three of six adventures teaches no
causality — it only advances the step and announces a count. Heal It even shows the findings under the
caption "Run the validation check to see what blocks launch…". Revealing the findings *on* the scan would
make the pre-launch-check concept land in one gesture.

**T6-9 — Operation buttons do not name their target (a11y).** The selection screen exposes six buttons
all named exactly `Start`; `ContentGraph` exposes three named `Stage install` plus one `Stage removal`;
Mod It's apply step and Take It Offline's re-check each expose two identically named buttons. A
screen-reader user navigating by button list cannot tell them apart. Undo It already demonstrates the fix
— `Restore Automatic return point (worlds not included)` carries both target and consequence in the name.
TERRA-5 saw the same `Stage removal` pattern on the live surface, so this is systemic to the shared
visuals, not Lab-only. Node buttons also announce as run-together text (`BetterCavesMod`,
`Purpose / nameUntitled`).

**T6-10 — Fix It's experiment panel reads "Awaiting your confirmation" after the experiment has run**, on
the same screen that displays its outcome (*"One item changed, launched once — the crash happened
again."*).

### P3 findings

- `Reset` leaves the previous feedback banner in place — Heal It returns to Step 1 still announcing
  "Scan complete: 1 blocker, 1 warning, 1 recommendation." while asking you to run the scan.
- Build It's first tray tile reads `Minecraft 1.20.1 / needs —`.
- Build It's rejection copy says Forge "does not fit 1.20.1 **for this mod**" at a step where no mod
  exists yet; it risks teaching that loaders are chosen per-mod.
- Health findings render `Loader · affects 0` / `Runtime · affects 0`; a zero count reads as a defect.
- The `+2 ~1 −1` diff shorthand in Undo It has no legend until you open a comparison, which then spells it
  out as `Added 1 · Changed 2 · Removed 0`.
- After Mod It applies, both BetterCaves and the now-removed Terrain Overhaul still show `conflicts 1`
  despite the socket reading `Resolved`.
- Heal It's loader rail disappears entirely once Fabric is chosen, so the result of the choice is no longer
  visible; only the blocker count and feedback confirm it.
- Carried forward from TERRA-3: the sidebar wordmark truncation at 1366x768 in the High readability preset.

### What is working, and should not be disturbed

Worth stating plainly, because the fixes above touch adjacent code:

- **Undo It is exemplary.** Scope is legible per return point (`worlds NOT included` vs `worlds included`),
  restore-while-running is blocked with adjacent inline feedback, the confirmation names the target, the
  undo point, and the world boundary, and the post-restore state visibly differs — the current label
  changes, a new `Undo restore point · Just now` appears, and the outcome panel confirms worlds were not
  touched. Every TERRA-3 P1 fix holds.
- **Fix It's epistemics.** `Strength is not certainty`, `One change at a time. One launch does not prove a
  cause.`, the visible `Candidate → Less likely` transition on a disproved hypothesis, `a new observation,
  not success`, and the completion's *"supporting — not proving — a cause"*. This is genuinely careful work.
- **Heal It's loader rail.** `1 satisfied · 0 uncertain · 2 failed` alongside `affects Tweakeroo, Sodium`,
  with `Uncertain is not compatible`, carries proven-vs-indeterminate visually rather than by assertion.
- **Mod It's dependency and apply behaviour.** The socket moving `Missing → Satisfied` while the "Missing
  requirements" region disappears is real visual causality, and the post-apply screen
  (`Plan applied ✓ Committed`, all proposal markers cleared, buttons flipped to `Stage removal`) makes
  staged-vs-executed unambiguous.
- **Build It's isolation.** `Your new instance` versus `Existing example — Separate · unchanged`, with the
  neighbour's counts never moving, answers TERRA-1's isolation question without prose.
- Simulation framing, progress persistence, and the handoff to real features all behave correctly: the
  banner is persistent, progress survives reload, and `My Instances` from a completion screen exits the
  simulation cleanly.

### Golden interaction standard — status

| Requirement | Status |
| --- | --- |
| Next action discoverable without paragraph instructions | Pass across all six adventures. |
| Cause and effect adjacent in space and time | **Fail** for T6-8 (effect precedes cause) and T6-3 (`Need attention` far from the loader row). |
| Current / proposed / rejected / applied visually distinct | Pass except **T6-3** (rejected renders as current). Heal It's `Automatic → Manual ◌ Proposed` is the model. |
| Dependencies, conflicts, and scope readable from the visual | **Fail on conflict** (T6-1). Dependencies and snapshot scope pass. |
| Serious actions state exact content and consequence | Pass for Undo It and Mod It's replacement; **fail** for Fix It's first experiment (T6-5). |
| Wrong predictions safe with visible feedback | Safe everywhere. **Fail on visible** for T6-7 (silent no-op). |
| Reset obvious, simulation boundary persistent | Pass (stale banner is P3). |
| Keyboard completion possible | Not verifiable through this harness; semantic buttons throughout, no drag-only task, `List view` linear equivalent present. |

---

## TERRA-6b — real-app verification of the live High Interaction surface (2026-08-11)

The two scope limits declared above were closed in a follow-up pass driving the **real Agora launcher**
(`d:\agora\target\debug\agora-desktop.exe`, the working copy's debug build) via computer use, against a
**real 136-mod instance** ("Copy of COBBLEVERSE", MC 1.21.1 / fabric 0.18.4, 1 real health warning).
Strictly read-only: no install, remove, restore, launch, repair, or sign-in was performed.

This is the first time the live surface has been seen with real data — TERRA-5 used a single-mod mock,
and mocks use fully-populated fixtures. Three of the findings below are invisible to any mock.

### T6-3 and T6-4 confirmed on real data

- **T6-3.** The instance bench renders `Loader   fabric` with **no compatibility marker of any kind** —
  no chip, no "Fits this setup", no "not verified". Visually identical to a loader Agora has confirmed
  compatible. Exactly as predicted from `InstanceBench.tsx:127` + `readAdapters/index.ts:68`.
- **T6-4 (point 3).** Clicking `Review health` returned to the Standard editor and the toggle now reads
  `High Interaction view` again — one review action permanently reverted the persisted preference to
  `standard`, on a real instance, as predicted.

### T6-11 (P1 → Sol) — a nullish-equality bug paints an entire healthy instance as warning

**Observed.** On one screen, simultaneously:

| Surface | Says |
| --- | --- |
| Instance bench summary | `136 Enabled · 0 Disabled · **0 Need attention**` |
| Health check panel | `0 blockers · **1 warning** · 24 recommendations` |
| Content graph | **every one of the 136 mod nodes carries a yellow `Warning` chip** |

The instance's single genuine warning is instance-level: *"The instance manifest tracks 1 enabled mod
file(s) that are absent from mods/: particular-1.1.2+1.21.jar.disabled."* — one mod. The graph flags all
136.

**Root cause.** `live/readAdapters/index.ts:100`:
`(health?.warnings ?? []).some((w) => w.filename === mod.filename || w.mod_id === mod.registry_id)`.
`HealthWarning.mod_id` is `string | null` (`lib/tauri.ts:711`) and `InstalledMod.registry_id` is
`string | null` (`:350`). An instance-level finding has `mod_id: null`; every non-curated mod has
`registry_id: null`; `null === null` is **true**. So every unattributed finding matches every
non-curated mod. On a 136-mod pack that is essentially the whole instance.

**The blocker arm is the same bug and is worse.** Line 97 does
`blocker.mod_id === mod.registry_id` identically. A single unattributed *blocker* would therefore render
**every non-curated mod as `blocked`**. This instance has 0 blockers, so it did not fire — that is luck,
not a safeguard.

**Why it matters, and why this is Sol's.** SOL-2 §15 BLOCKER 1 was about never letting uncertainty read
as healthy. This is the same truthfulness contract failing in the other direction: healthy content reads
as damaged, and the surface contradicts *itself* twice on a single screen. A user opening High Interaction
on a real modpack is told their instance is entirely warning-ridden. It also makes the `Need attention`
stat and the graph unfalsifiable against each other, which undermines the whole current-state story the
mode exists to tell.

**Required fix.** Compare identities only when both sides are non-null, and attribute instance-level
findings to the instance rather than to content nodes. Add a regression fixture with `mod_id: null` +
`registry_id: null` — every existing test uses populated ids, which is why this survived five Sol gates,
243 e2e tests, and TERRA-5.

### T6-12 (P2) — every health finding prints its message twice, and recommendations are all "Runtime"

`healthToVisual` sets `title` and `summary` to the same string for all three severities
(`readAdapters/index.ts:172–173`, `184–185`, `195–196`), so each finding renders its full sentence
bold and then again in body text. With 24 recommendations the panel is 48 identical lines.

The same function hardcodes `structuredKind: 'runtime'` for **every** recommendation (`:199`) and
`affectedIds: []` (`:197`). Content/dependency recommendations such as *"'AdvancementPlaques…jar'
recommends 'jade' but it is not installed"* therefore display as `Runtime · affects 0`. The category is
wrong and the count is always zero.

### T6-13 (P2) — content nodes are labelled with raw jar filenames

`readAdapters/index.ts:105` sets `name: mod.filename`, so the graph reads
`AdvancementPlaques-1.21.1-fabric-1.6.8.jar`, `BetterF1-Fabric-1.1+1.21.7.jar`. The Standard editor
directly behind it lists the same mods as **Advancement Plaques**, **Better F1 Reborn**, **Cobblemon
Capture XP**, with the filename as a secondary line. The findings text carries the same raw filenames and
internal ids (`'waila'`, `'prism'`, `'toastmanager'`).

This inverts the premise of the mode. High Interaction is meant to be the friendlier, lower-text, more
visual presentation; on real data it is **strictly less legible than the Standard view it replaces**, and
it surfaces exactly the kind of internal identifier the shared contract says to keep hidden from users.

### T6-14 (P3) — two identical "Use Standard view" buttons

`InstanceEditor` renders its own escape button and `LiveInteractiveHost` renders another; both are
primary-styled and stack vertically at the top right. During the initial `Loading live data…` state they
are the only two controls on screen.

### Observation — the mode currently offers one content verb, and it is the destructive one

With install/update blocked (§19.5) and enable/disable rejected, the only action on all 136 content nodes
is a red `Stage removal`. The flagship "playful, visual" surface is, on real data, a wall of remove
buttons. That is a faithful consequence of the approved-seam list rather than a defect, but it is worth a
product decision before release: the mode reads as a bulk-removal tool.

### What passed on real data

- **Large-instance handling holds.** The 12-node spatial cap works and offers `Show all 136 items (124
  more)`; the surface loaded a 136-mod instance without stalling.
- **The health bridge is correct end-to-end.** `Review health` left High Interaction, opened the Standard
  reviewOnly `HealthDialog` for the right instance, and that dialog contained **zero** Disable / Fix /
  Repair controls — only `Close` and the `Show on next launch` toggle. §18.5 holds on real data.
- **Refresh and the Standard escape are present and functional**; `Back` returns cleanly.
- The `Live` / `High Interaction` badges and the honest header copy render as TERRA-5 described.

---

## TERRA-7 — retest of the TERRA-6 fix batch (2026-08-11)

**Verdict: PASS. All five P1 findings are closed and verified.** Retested black-box against the real
launcher (debug build on the dev server) with the same 136-mod instance, plus the Lab played from a clean
start. No source inspection was needed to confirm any fix — every one was directly visible.

| Finding | Retest result |
| --- | --- |
| **T6-1** conflict invisible on the played path | **Pass.** Playing Mod It straight through to "A conflict surfaced" now renders the amber **Conflicts** region, a `Conflicts with Terrain Overhaul · Blocking` socket on BetterCaves, and `Blocker` + `conflicts 1` on **both** endpoints. The played path and the resumed path finally teach the same lesson. |
| **T6-2** resume reverses a decision | **Pass.** A resumed Mod It scene no longer asserts the optional; it re-offers `Include optional` / `Skip optional` instead. Fix It and Take It Offline now clamp resume to their last unambiguous checkpoint rather than inventing a hypothesis or a prepare/leave choice. |
| **T6-3** unmarked loader compatibility | **Pass.** The real instance's bench now reads `Loader  fabric  Not verified`. Incompatible reads `Does not fit this setup` in destructive styling and indeterminate reads `Needs review — not proven for this setup`, so uncertainty and incompatibility are distinct from each other and from confirmed-good. In Build It, choosing Forge on 1.20.1 no longer writes it into the bench: it renders as a proposal marked `Incompatible` with `proposed loader choice`, and the step's own rule ("an incompatible tile cannot become current state") is finally true. |
| **T6-4** no setting, self-disabling mode | **Pass.** Settings → Appearance now carries an `Instance view` control. Editing an instance opens **directly** into High Interaction when that is the preference. Critically: using `Review health` — which previously reset the preference permanently — leaves High Interaction for the Standard dialog and the setting **still reads High Interaction** afterwards. §18.4's unmount guarantee is intact and visibly unchanged. |
| **T6-11** false mass warnings | **Pass.** The same instance that showed 136 false `Warning` chips now shows **zero**, consistent with its own `0 Need attention` and `1 warning` summaries. The single real warning is attributed to the instance, not sprayed across content. |

Secondary findings retested: **T6-7** Core Lib now shows `Already proposed — review it in the staging
dock.` instead of a silent no-op button. **T6-12** findings render a distinct headline (`Manifest drift`,
`Missing optional dependency — Advancement Plaques`) above their body, recommendations are categorised
`Content` rather than `Runtime`, and `affects 0` is replaced by `this instance` or a real count.
**T6-13** nodes read as `Advancement Plaques` with `AdvancementPlaques-1.21.1-fabric-1.6.8.jar` retained
beneath. **T6-14** one escape button. **T6-8** health findings no longer appear before the scan that
produces them. **T6-9** `Start Build It`, `Stage removal: Cobblemon` — operation buttons name their
target. Both SOL-3 P2s are closed: the Quilt message now names what review means, and the disproved
memory hypothesis explains why the out-of-memory line was a symptom rather than the cause.

**Golden interaction standard — now met.** The two rows that failed at TERRA-6 pass: cause and effect are
adjacent (the scan reveals its own findings; the rejected loader is marked at the loader row), and
conflicts are readable from the visual rather than from prose. Serious actions name their exact change —
Fix It's first experiment now says what it will do for *that* hypothesis instead of always claiming it
disables a mod.

**Residual, non-gating:** the `recommends` socket still reads `Needs review` after a player deliberately
skips the optional — the relationship-state vocabulary has no "declined" value, and adding one is a
domain-contract change I would not make for a P3. Recorded for Phase 2. The TERRA-3 wordmark truncation
also remains open.

### Handoff

```text
Agent: Terra
Phase: TERRA-6 final UX + TERRA-6b real-app verification + TERRA-7 retest
Commit / branch / dirty status: master; uncommitted. Existing interactive work remains uncommitted.
Files changed:
  docs/interactive/UX_FINDINGS_TERRA.md   (this TERRA-6 section)
  docs/interactive/IMPLEMENTATION_STATUS.md (status row + TERRA-6 result)
  .claude/launch.json                     (dev-server preview config; incidental)
Tests run:
  No automated suite run (review pass). Black-box play of all six adventures from a clean profile in the
  local Vite preview, plus DOM/a11y-tree inspection and targeted source reads to locate seams.
  TERRA-6b: read-only computer-use pass over the REAL launcher (d:\agora\target\debug\agora-desktop.exe)
  against a real 136-mod instance — bench, content graph, health lens, Review health bridge. No install,
  remove, restore, launch, repair, or sign-in performed.
How to launch/test:
  cd desktop; npm run dev; sidebar -> "Agora Lab".
  T6-1 repro: play Mod It straight through to "A conflict surfaced" (conflict is prose-only), then
    Exit -> Resume at the same step (full Conflicts region appears).
  T6-2 repro: choose "Skip optional textures", Exit, Resume -> Nice Textures is Satisfied/proposed.
  T6-3 repro: Build It -> place 1.20.1 -> "Choose Forge" -> bench Loader shows Forge unmarked.
  T6-7 repro: Mod It -> stage BetterCaves -> snap Core Lib -> click Core Lib's "Stage install" (no-op).
Known failures:
  NONE OPEN. All 5 P1 (T6-1, T6-2, T6-3, T6-4, T6-11) closed and retested at TERRA-7, plus T6-7, T6-8,
  T6-9, T6-12, T6-13, T6-14 and both SOL-3 P2s. Residual non-gating: a skipped optional recommendation
  still reads "Needs review" (no `declined` relationship state exists — Phase 2); TERRA-3 wordmark
  truncation; full viewport/density/contrast matrix still owed to Luna.
Decisions made:
  TERRA-6 does NOT clear the work for SOL-4. Assessment is user-facing meaning, teaching, and safety
  presentation only; no code was changed and no shared visual-domain contract was altered.
Decisions explicitly not made:
  No fixes implemented (Terra does not rewrite the system). No judgement on whether T6-3's live-surface
  reach is a SOL-2 regression — that is Sol's call. No colour/contrast/density re-verification (no
  screenshots this pass); no live High Interaction re-test (no backend in the browser preview).
Required next agent:
  Sol first, for T6-3 (shared InstanceBench renders every non-compatible loader state as unmarked, which
  reaches real instances) and for the T6-4 session-state vs persisted-preference split under §18.4.
  Then DeepSeek for T6-1, T6-2, the P2/P3 list, and the two open SOL-3 P2s. Luna's final regression should
  cover the colour/contrast/density surface this pass could not, plus the T6-1 resume-vs-play divergence.
Why work is stopping:
  TERRA-6 is complete and the P1s belong to Sol and DeepSeek, not to Terra.
```

## Handoff (TERRA-5)

```text
Agent: Terra
Phase: TERRA-5 — live High Interaction deep review
Commit / branch / dirty status: master; uncommitted. Existing shared interactive work remains uncommitted.
Files changed:
  docs/interactive/UX_FINDINGS_TERRA.md (TERRA-5 live-review section appended; handoff updated)
Tests run:
  Black-box local Vite run against the full read-command mock (high-interaction.spec.ts shape).
  Drove: My Instances -> Edit -> High Interaction view; Review health -> Standard reviewOnly
  HealthDialog (0 Disable/Fix buttons); Stage removal -> canonical InstallFlow (resolve_install_plan
  only; Remove Safely/Cancel; Cancel issues no apply); Refresh (fresh re-read, scene stays visible);
  List view; keyboard focus; Use Standard view escape; document-overflow check at 1024px.
  Rejected/blocked seams probed and confirmed absent: 0 memory / 0 restore / 0 disable / 0 compare /
  0 crash / 0 offline buttons.
How to launch/test:
  cd desktop; npm run dev; My Instances -> Edit -> High Interaction view. Approved seams and the
  rejected-seam absences are documented in the TERRA-5 tables above.
Known failures:
  No P0/P1. P2: Runtime workbench "System Java / Unknown" chip is ambiguous beside a confident memory
  recommendation (non-gating). P3: transient first-paint right-edge crowding at narrow widths (non-gating;
  no persistent overflow). Carried-forward P3 shell wordmark truncation remains a non-gating follow-up.
Decisions made:
  TERRA-5 clears the live High Interaction surface for SOL-3. All reachable approved seams preserve
  backend authority, current/proposed distinction, and destructive seriousness; all rejected/blocked
  seams are genuinely absent. Assessment is user-facing meaning and safety presentation only; no shared
  visual-domain contract was changed.
Decisions explicitly not made:
  No code fixes, no architecture changes, no enabling of any blocked/rejected seam (install/update,
  restore, memory, disable, offline), no P2/P3 polish implementation.
Required next agent:
  Sol for SOL-3 educational correctness. The P2 Runtime "Unknown" legibility note and the P3 first-paint
  crowding are queued for DeepSeek as non-gating polish; Luna may fold the first-paint overflow check
  into routine regression.
Why work is stopping:
  TERRA-5 is complete; the coordination sequence hands educational-correctness review to Sol (SOL-3)
  before the final Terra/Luna pass and DeepSeek final fixes.
```
