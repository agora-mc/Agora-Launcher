# Interactive Visual Language

Status: SOL-0 visual-semantics contract

Audience: implementation, UX review, accessibility review, and lesson authors

The interactive experiences should feel playful without making consequential behavior casual. Visuals answer “what depends on what, what is true now, what might change, and how can I recover?” The same meaning must survive without color, animation, dragging, or a spatial canvas.

## 1. Core grammar

### 1.1 Current versus proposed

- **Current** uses a solid container, a visible `Current` label when comparison is active, and the normal semantic reading order.
- **Proposed add/change** uses an outlined staging container, a `Proposed` label, and a concise change summary.
- **Proposed removal/disable** remains in place with a removal/disable marker and readable name. It does not simply fade away.
- **Applying** uses a busy state plus textual progress from the authoritative operation.
- **Committed** appears only after backend success and a fresh read; it becomes current rather than lingering as a celebratory overlay.
- **Rejected/cancelled/rolled back** returns to a refreshed current state and announces the outcome.

Color may reinforce these states but never defines them. Shape, border treatment, icon, text label, and assistive description carry the meaning.

### 1.2 Status vocabulary

Use a small consistent vocabulary:

- **Ready** — Agora has verified the relevant condition.
- **Needs attention** — the player can continue only after review or a choice.
- **Blocked** — an authoritative blocker prevents the operation.
- **Recommended** — useful guidance that does not itself block.
- **Unknown** — Agora cannot currently verify the fact.
- **Busy** — an authoritative operation owns the resource.
- **Locked** — editing is disallowed by player policy or a current operation.

Do not substitute “safe,” “guaranteed,” “fixed,” “the cause,” or “offline ready” unless the underlying authority supports exactly that claim.

## 2. Recurring visual meanings

| Pattern | Meaning in motion-capable view | Persistent/non-color cue | Reduced-motion equivalent | Keyboard and screen-reader equivalent |
|---|---|---|---|---|
| Socket and snapping | A proposed item approaches a compatible requirement socket; accepted simulation choices settle into place. | Matched shape, connector label, and `Fits requirement` text. | Item appears in the proposed socket without travel; a short status message names the match. | Focus the item, choose `Place`, then choose a named socket from a list; announce the match and whether it is proposed or current. |
| Dependency line | A labeled line connects content that requires or recommends another item. | Line style plus `Requires` or `Recommends` badge at both the graph and detail view. | Static line/state change. | Relationship list grouped under each node, navigable with arrow keys and available as ordinary links/buttons. |
| Broken requirement | A required relationship ends at an open/broken socket and stops the validation path. | Break glyph, `Missing requirement` label, affected-item count, and blocker semantics. | Immediate blocker reveal with focus/announcement. | `Missing requirements` region lists source, missing target, consequence, and review action. |
| Conflict | Two items repel or a line intersects a conflict mark during simulation. | `Conflicts with` text, crossed connector, and blocker/warning label supplied by the plan. | Static conflict mark and message. | Relationship/details list exposes the same conflict and choices; no hover is required. |
| Validation sweep | A causal scan advances through visible checks after the player requests validation. | Ordered checklist remains after the sweep, with each result labeled. | Checklist updates in order or all at once; announce one summary rather than every decorative step. | `Validate` button starts the same simulated/read-only action; focus moves to a summary, then individual findings. |
| Recovery ghost | A translucent but outlined prior arrangement shows what a return point would restore. | `Return point` label, timestamp/role, scope badges, and current/proposed comparison. | Static side-by-side or before/after summary. | `Compare` opens a structured list of added/changed/removed items and the world/save boundary. |
| Recovery timeline | Return points appear in time order; current state is separately pinned. | Roles such as `Current known good`, `Known good`, and `Undo restore`; scope text on every item. | No travel along the timeline; focus/selection changes instantly. | Ordered list with role, date, scope, compare, and restore-review actions. |
| Staging area | Local choices collect separately before review. | Bordered `Proposed changes` region with counts, undo controls, and `Review in Standard flow`. | Identical static region. | Landmark/heading, ordered change list, remove-from-proposal buttons, and one review action. |
| Lock | Controls visually close or receive a lock plate when an operation/player lock applies. | Lock icon plus `Locked`/`Busy` text and reason; controls retain readable names. | Immediate state change. | Disabled semantics only when truly unavailable; adjacent text explains reason and where focus can go next. |
| Disabled content | Content remains visible but marked off. | Explicit `Disabled` badge and toggle state; name remains fully readable. | No fade animation. | Switch/button exposes checked state and dependency consequences before confirmation. |
| Unavailable action | An action cannot currently be offered because data or capability is absent. | `Unavailable` text and reason, distinct from disabled content. | Same. | Prefer an enabled explanation/details control; if a control is disabled, put the reason in persistent text discoverable before it. |

## 3. Sockets and snapping

Sockets are an educational metaphor, not a claim about literal file placement.

- A socket represents a named requirement or supported choice.
- Shape families can distinguish required, recommended, and conflicting relationships, but the text label is mandatory.
- Snapping in Lab means “this simulated choice satisfies the shown condition.”
- In live mode, snapping can stage a proposal only. It must stop in the staging state and open the existing review flow before any operation.
- An uncertain loader/content relationship never snaps with the same treatment as a proven compatible one; it lands in an `Needs review` slot.
- A missing or ambiguous target stays open. The visual must not silently choose a dependency.

Drag is optional input sugar. Click/tap-to-select plus `Place`, and keyboard `Move to…`, are the primary equivalent commands.

## 4. Relationships and dependency simplification

The graph teaches three player-facing relationships: requires, recommends, and conflicts with. It should not expose raw loader metadata syntax or provider-specific dependency declarations.

When a graph is dense:

- show direct relationships for the focused item;
- collapse transitive groups into a labeled count;
- let the player expand a group deliberately;
- keep missing requirements and blockers visible regardless of filtering;
- provide a complete searchable linear relationship view;
- avoid implying that vertical position is install order unless explicitly labeled.

A line is evidence of a known relationship, not evidence that an item is healthy. Health is a separate visual layer.

## 5. Validation and health

The validation sweep communicates causality only when the player has requested a simulated check or a read-only live health scan is already authorized by the host.

Results use the existing health hierarchy:

1. blockers;
2. warnings;
3. recommendations.

Recommendations never acquire blocker styling. Loader findings use structured compatibility evidence; presentation code must not interpret prose to select a candidate or action.

The sweep ends with a persistent summary. A green flourish may celebrate a Lab checkpoint, but live launch readiness is conveyed by the authoritative health result and action availability—not the flourish.

## 6. Recovery language

Recovery visuals should make reversibility understandable without promising more coverage than exists.

- Keep **current state**, **current known good**, **other known-good points**, and **undo restore** visually and verbally distinct.
- Every snapshot/return point names its scope and whether worlds/saves are included.
- A pre-change return point can be described as “Agora can return these instance files to this state,” not “everything is backed up.”
- Restore is never a playful drop gesture that executes immediately. A gesture selects a candidate; a serious comparison and confirmation follow.
- Crash Doctor experiments show a recovery point before the first mutation and show restoration when an experiment fails or is abandoned.

Recovery ghosts are comparisons, not live filesystem previews. If the diff is unavailable, render `Comparison unavailable` rather than an empty ghost.

## 7. Crash evidence language

The Crash Evidence Board is an investigation metaphor:

- cards are **clues**;
- suspects are **hypotheses**;
- an experiment can make a hypothesis more or less likely;
- one successful launch requires player confirmation and does not prove a cause;
- a changed crash is a new observation, not success;
- lack of evidence is `inconclusive`, not `healthy`.

Avoid courtroom verdicts, certainty percentages, “culprit,” or celebratory success before the existing Crash Doctor records its outcome and recovery behavior.

## 8. Runtime and memory language

The Runtime Workbench should show:

- what Agora is choosing automatically;
- the current effective memory choice;
- the recommended region and why;
- the proposed manual choice and remaining system headroom;
- whether the runtime satisfies the game’s required Java generation.

It should not default to raw JVM flags. Manual controls must not imply that more memory is always better. Automatic and manual are peer choices with clear consequences, and a proposed setting stays staged until the existing save path confirms it.

## 9. Network readiness language

The map uses player-visible needs: game files, loader, content, Java, and sign-in/launch-mode needs.

- `Ready` means a core-owned check verified the category for the selected instance and launch mode.
- `Blocked by policy` means a network policy prevents a needed fetch; it is not the same as missing content.
- `Unknown` means Agora cannot truthfully verify readiness from current data.
- A cached registry never lights all readiness nodes by itself.
- A Lab success teaches the checklist; it does not assert that the player’s real instance is ready.

## 10. Motion contract

Animation exists only to show one of these causal relationships:

- a decision creates a proposal;
- a requirement becomes satisfied or broken;
- validation progresses through checks;
- a backend progress event updates a pending operation;
- a recovery operation returns the view to refreshed current state.

Animation must not:

- start an operation;
- advance a lesson on completion;
- imply backend success before confirmation;
- conceal a blocker or removed item;
- loop as decoration near a serious choice;
- move focus or reorder the accessibility tree unpredictably.

Existing system/reduced/full motion preferences remain authoritative. Under reduced motion, change state immediately, retain the same labels and focus target, and announce the causal result concisely.

## 11. Keyboard and focus model

Every canvas has a nearby `List view` or a unified mode switch that does not lose selection.

- `Tab` reaches major regions and actions rather than every decorative node.
- Arrow keys move among nodes inside a graph/list composite.
- `Enter` or `Space` inspects/selects; a separately named command stages a change.
- `Escape` cancels a local proposal or closes a detail surface, never an unconfirmed backend operation silently.
- After refresh, focus returns to the equivalent entity if it exists, otherwise to the scene heading with an explanation.
- Opening an existing review dialog moves focus using that dialog’s established behavior; closing it returns focus to the originating intent control.
- Live regions announce summaries (`3 checks complete, 1 blocker`) rather than streaming decorative movement.

No outcome depends on pointer precision, drag distance, hover duration, animation timing, or audio.

## 12. High contrast, scale, and density

Use Agora semantic tokens and established appearance attributes. Do not hard-code a second theme inside the feature.

- At 200% text, actions wrap below content and spatial diagrams can yield to the linear view.
- High contrast keeps outlines, connector styles, state labels, and focus rings visible.
- Compact density may reduce whitespace, not target size or explanatory state labels.
- Node names and serious action labels do not truncate without an accessible way to read them.
- Patterns/dashes/shape and text reinforce state when colors converge.

## 13. Review checklist for a new visual

A new visual pattern is acceptable only if reviewers can answer yes to all of these:

1. Can a player tell current from proposed without color or motion?
2. Does the same action exist without dragging or a canvas?
3. Does reduced motion preserve cause and outcome?
4. Does the visual wait for authoritative live success?
5. Are unknown, blocked, recommended, and unavailable distinct?
6. Does it avoid exposing implementation trivia as a lesson?
7. Does focus remain understandable after proposal, review, rejection, and refresh?
8. Does it stay legible at high contrast and 200% text?
