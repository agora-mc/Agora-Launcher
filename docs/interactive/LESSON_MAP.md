# Agora Lab Lesson Map

Status: SOL-0 educational architecture

Scope: the six required Lab adventures

Each adventure is short, deterministic, simulated, low-text, and recoverable. It teaches one player decision pattern through visible cause and effect. Lab completion never changes a real instance and never asserts that the player’s real setup is healthy, recoverable, or offline-ready.

## 1. Shared lesson shape

Every adventure follows the same rhythm:

1. **See the situation** — a small scene and one plain-language goal.
2. **Try a decision** — spatial gesture, click/tap command, or keyboard/list equivalent.
3. **See the consequence** — labels and causal visuals show what became satisfied, blocked, uncertain, or recoverable.
4. **Correct safely** — at least one common mistake can be undone inside the simulation when educationally useful.
5. **Name the mental model** — one short takeaway, not an implementation lecture.
6. **Choose what is next** — replay, open the Field Guide, or leave Lab and navigate to the relevant real Agora destination.

Checkpoints are awarded for decisions and corrections, not for animation completion or time spent.

## 2. Build It

### User situation

“I want a separate place for a particular Minecraft setup, but I do not know what choices define it.”

### Mental model

An instance is an isolated home for a Minecraft version, loader choice, content, settings, and recovery history. Choosing the basic foundation first prevents unrelated setups from becoming tangled.

### Interaction

- Place a game version tile on an empty Instance Bench.
- Choose a loader family that fits the intended content, or choose vanilla when no loader is needed.
- Give the simulated instance a recognizable purpose/name.
- Try one intentionally incompatible content tile and observe that it cannot become a valid current state.
- Keyboard/list equivalent: select each foundation field from named choices and review the assembled summary.

### Success condition

The simulated instance has a named purpose, game version, and compatible loader/vanilla choice, and the player can identify that it remains separate from another example instance.

### Intentionally not taught

- directory layout or manifest schema;
- registry compilation/signatures;
- loader installation internals;
- JVM arguments;
- every advanced creation option.

### Field Guide destination

- `instances` — **Instances: Your Isolated Worlds**
- optional follow-up: `modding-foundations` — **Modding Foundations**

### Real Agora destination

My Instances -> Create Instance. The Lab scene is discarded before navigation; no simulated choices prefill a real creation operation in the first implementation.

## 3. Mod It

### User situation

“I found a mod I want, but it needs other content and may conflict with what is already installed.”

### Mental model

Installing, updating, or removing content is a planned change. Requirements, recommendations, conflicts, file changes, and a recovery point should be reviewed together before current state changes.

### Interaction

- Stage a desired mod on the Content Graph.
- Snap a required dependency into its named socket.
- Decide whether to include an optional recommendation.
- Encounter a simulated conflict and choose between keeping current content or changing the proposal.
- Review a Current versus Proposed staging summary before applying the **simulated** plan.
- Keyboard/list equivalent: choose `Stage`, resolve named required/optional/conflicting relationships, then review the ordered proposal list.

### Success condition

All required relationships are satisfied, blocking conflicts are resolved, optional choices are deliberate, and the player reviews current versus proposed state before simulated application.

### Intentionally not taught

- provider APIs, download URLs, or hashes;
- receipt and plan-fingerprint schemas;
- file materialization or transaction implementation;
- raw dependency metadata syntax;
- an exhaustive modpack-authoring workflow.

### Field Guide destination

- `install-update` — **Installing, Updating & Removing Content**
- optional follow-up: `modding-foundations` — **Modding Foundations**

### Real Agora destination

Browse with a selected target instance, then the existing Install Flow. A later live gesture may only open that canonical flow with an intent; it may not apply the Lab plan.

## 4. Heal It

### User situation

“My instance is not ready to launch, and I need to understand whether the issue is content, loader compatibility, Java, or merely a recommendation.”

### Mental model

Agora’s pre-launch health check separates blockers, warnings, and recommendations. Loader and Java choices should be based on structured compatibility and the selected game—not guesswork or “newest is always best.”

### Interaction

- Run a simulated validation sweep over content, loader, runtime, and recovery readiness.
- Inspect one blocker, one warning, and one recommendation with visibly different consequences.
- Move a loader candidate onto the Loader Rail and distinguish proven-compatible from indeterminate.
- Choose automatic memory/runtime guidance or stage a conservative manual choice with visible headroom.
- Re-run the simulated check after correcting the blocker.
- Keyboard/list equivalent: validate, open categorized findings, choose a candidate/action, then validate again.

### Success condition

No simulated blocker remains, the player can explain that warnings require review while recommendations do not block, and the chosen loader/runtime state is compatible or explicitly marked uncertain.

### Intentionally not taught

- parsing health-message text;
- loader catalog ranking internals;
- Java download/install mechanics;
- garbage-collector flags or raw JVM arguments;
- bypassing blockers as a normal fix.

### Field Guide destination

- `launching` — **Launching & Process Control**
- `java-performance` — **Java & Performance**

### Real Agora destination

Selected instance -> Health/Launch review, Loader Chooser, or Instance Editor -> Java. Existing health, loader, and settings controllers remain authoritative.

## 5. Fix It

### User situation

“The game crashed, and I want to test a likely explanation without making the instance worse.”

### Mental model

Crash Doctor forms hypotheses from local evidence and changes one bounded variable at a time. A recovery point comes before the first experiment. Outcomes can strengthen, weaken, or leave a hypothesis inconclusive; they do not automatically prove a cause.

### Interaction

- Arrange simulated evidence cards by relevance.
- Select a likely hypothesis, with strength expressed as low/medium/high rather than certainty.
- Preview the dependency-aware content that a test would disable.
- Confirm that a simulated recovery point exists, then run the one-variable experiment.
- Observe one branch where the crash changes or continues and revise the hypothesis; observe/replay a successful branch that still asks for player confirmation.
- Cancel an experiment and see the simulated current state restored.
- Keyboard/list equivalent: evidence list -> hypothesis list -> experiment summary -> run/cancel -> outcome review.

### Success condition

The player runs or safely cancels a bounded experiment, preserves recovery, and identifies the result as supporting, contradicting, or inconclusive rather than “proven cause.”

### Intentionally not taught

- crash-signature regexes;
- raw log paths or full logs by default;
- AI as an authority;
- disabling many unrelated items at once;
- a claim that one successful launch proves causality.

### Field Guide destination

- `crash-recovery` — **Crash Recovery & Troubleshooting**

### Real Agora destination

Selected instance -> Troubleshoot / Crash Doctor. The existing investigator owns evidence collection, recovery snapshot timing, dependency planning, relaunch, outcome correlation, and restoration.

## 6. Undo It

### User situation

“A change made my setup worse, and I need to understand which return point can help—and what it does not protect.”

### Mental model

Snapshots and Last Known Good are scoped return points for instance state. Restore is a consequential operation with a comparison and an undo return point. Worlds/saves are protected only when the selected snapshot’s scope explicitly includes them; external world backups remain a separate responsibility.

### Interaction

- Explore a simulated Recovery Timeline with current state, current known good, another snapshot, and undo restore.
- Select a recovery ghost to compare added, changed, and removed content.
- Choose between a fast automatic return point that excludes worlds and a broader manual example that includes them.
- Attempt to restore while the simulated game is running and see the action remain blocked.
- Stop the simulated process, review the world/save boundary, seriously confirm, and see a simulated pre-restore undo point created.
- Keyboard/list equivalent: ordered return-point list -> comparison -> scope confirmation -> restore simulation.

### Success condition

The player chooses an appropriate return point, states whether worlds/saves are included, stops the active simulated process, and completes the serious confirmation with an undo point visible.

### Intentionally not taught

- content-addressed storage or object hashes;
- snapshot manifest schema/fingerprints;
- retention implementation details;
- the false claim that every snapshot is a complete backup;
- one-click restore from the playful timeline.

### Field Guide destination

- `snapshots-loadouts` — **Snapshots, Loadouts & Recovery**

### Real Agora destination

Instance Editor -> Snapshots. High Interaction Mode may select and compare a snapshot, but the existing backend restore plus a new serious live review/confirmation bridge must remain authoritative and requires SOL-2 approval.

## 7. Take It Offline

### User situation

“I will lose internet access and want to know what this particular instance and launch mode will still need.”

### Mental model

Offline readiness is instance- and launch-mode-specific. The catalog being cached does not mean the game, loader, content, Java, or sign-in/launch needs are all ready. Network policy can also block a missing fetch. Unknown must remain unknown.

### Interaction

- Inspect a simulated Network Readiness Map with game files, loader, content, Java, and sign-in/launch-mode needs.
- Find a missing content artifact and a separate category blocked by policy.
- Choose the correct preparation action, re-check the simulation, and distinguish `Ready` from `Unknown`.
- Toggle between delegated and direct example scenarios to see that sign-in/launch needs differ.
- Keyboard/list equivalent: readiness checklist -> issue details -> simulated preparation choice -> re-check.

### Success condition

Every simulated required category is verified ready, and the player can explain why cached registry metadata alone is insufficient and why an unknown category cannot be promised.

### Intentionally not taught

- internal cache paths, receipts, or artifact hashes;
- how network request gating is implemented;
- a universal “offline forever” guarantee;
- using a test launch as a read-only readiness check;
- that Lab completion says anything about the player’s live instance.

### Field Guide destination

- `privacy-offline` — **Privacy, Network Access & Offline Play**

### Real Agora destination

Settings -> Privacy plus the selected instance’s launch preparation/health surfaces. A complete live visual is unavailable until `agora-core` exposes a truthful read-only aggregate readiness query; Standard Mode and the Guide must not claim otherwise.

## 8. Progression and prerequisites

Adventures may be taken in any order, but the suggested path is:

```text
Build It -> Mod It -> Heal It -> Fix It
                      |          |
                      v          v
               Take It Offline  Undo It
```

This is guidance, not a gate. A player seeking crash or recovery help should be able to open Fix It or Undo It directly. Each adventure contains the small amount of prerequisite context it needs.

## 9. Educational review gates

Before an adventure is accepted:

- Terra confirms that the intended cause/effect is perceptually understandable without explanatory paragraphs.
- Accessibility tests confirm the same decisions and feedback in the linear/keyboard path.
- Sol later confirms that dependency, loader, Java, recovery, Crash Doctor, and offline claims remain technically correct.
- A scenario contract confirms all outcomes are simulated and no real authority enters Lab.
- The final takeaway fits in one short statement and links to the relevant existing Field Guide topic.
