# Interactive Presentation Domain Models

Status: SOL-0 contract

Purpose: define the smallest shared language needed by Lab and High Interaction Mode

These are presentation models, not persistence schemas, command payloads, or copies of Rust/Tauri DTOs. Names and example TypeScript shapes are normative at the concept level; DeepSeek may make small mechanical refinements during the vertical slice, but changing authority, safety, or meaning requires a Sol review.

## 1. Modeling rules

1. Include only facts a player can see, decide about, or use to recover.
2. Preserve uncertainty. `unknown` and `indeterminate` are valid outcomes, not errors to hide.
3. Keep backend authority data private to `live/`: hashes, plan fingerprints, scan tokens, receipts, paths, and raw manifests never enter a visual model.
4. Carry current and proposed values together where a choice is being staged.
5. Use stable opaque IDs for focus and relationships, never file paths as identity.
6. Make simulation versus live origin explicit on every scene.
7. Prefer small summaries and counts; obtain full detail through deliberate disclosure.
8. Do not convert a backend recommendation into a requirement or a hypothesis into a cause.

## 2. Shared primitives

```ts
type VisualId = string;

type ExperienceSource =
  | { kind: 'simulation'; scenarioId: string; scenarioVersion: number }
  | {
      kind: 'live';
      viewRevision: string;
      observedAt: string;
      freshness: 'fresh' | 'refreshing' | 'stale' | 'unknown';
    };

type Knowledge = 'known' | 'unknown' | 'unavailable';
type Availability = 'available' | 'busy' | 'locked' | 'unavailable';
type Severity = 'blocker' | 'warning' | 'recommendation';
type Compatibility = 'compatible' | 'indeterminate' | 'incompatible' | 'unknown';

interface VisualValue<T> {
  current: T;
  proposed?: T;
}

interface VisualScene {
  source: ExperienceSource;
  instance?: VisualInstance;
  content: VisualContentNode[];
  relationships: VisualRelationship[];
  findings: VisualHealthFinding[];
  proposals: VisualProposal[];
}
```

`viewRevision` is a local read-set identifier used to detect that the player acted on an older scene. It is not a backend plan fingerprint. Live controllers keep operation-specific authority data private and re-read before opening a review flow.

Simulation IDs should be visibly namespaced in fixtures, such as `lab:mod:map`, so tests can reject accidental mixing. The UI normally shows names, not IDs.

## 3. Proposal state

```ts
interface VisualProposal {
  id: VisualId;
  intent: VisualIntent;
  phase: 'proposed' | 'in-review' | 'applying' | 'rejected';
  title: string;
  summary: string;
  destructive: boolean;
}
```

A committed proposal is removed after a fresh authoritative read. “Committed” is an event/outcome, not a permanent overlay that can drift from current state.

## 4. VisualInstance

```ts
interface VisualInstance {
  id: VisualId;
  name: string;
  gameVersion: string;
  loader: VisualValue<{
    family: string;
    version?: string;
    compatibility: Compatibility;
  }>;
  lockState: 'editable' | 'locked-by-player' | 'busy';
  recoveryReadiness: 'ready' | 'preparing' | 'failed' | 'unknown';
  launchState: 'idle' | 'starting' | 'running' | 'stopping' | 'delegated' | 'failed';
  contentSummary: {
    enabled: number;
    disabled: number;
    needsAttention: number;
  };
}
```

What is omitted: instance paths, full manifests, raw launch configuration, snapshot object metadata, Java paths, and lock implementation details.

## 5. VisualContentNode

```ts
interface VisualContentNode {
  id: VisualId;
  name: string;
  kind: 'mod' | 'modpack' | 'resource-pack' | 'shader' | 'datapack' | 'world';
  version?: VisualValue<string | null>;
  presence: VisualValue<'installed' | 'not-installed'>;
  enabled: VisualValue<boolean>;
  health: 'healthy' | 'needs-attention' | 'blocked' | 'unknown';
  relationshipSummary: {
    requiredBy: number;
    requires: number;
    conflicts: number;
  };
  availability: Availability;
}
```

The node describes player-relevant state. Download URLs, hashes, receipt records, internal artifact paths, raw metadata, and provider-specific implementation fields remain outside the model.

## 6. VisualRelationship

```ts
interface VisualRelationship {
  id: VisualId;
  fromId: VisualId;
  toId?: VisualId;
  kind: 'requires' | 'recommends' | 'conflicts-with';
  state: 'satisfied' | 'missing' | 'conflicting' | 'indeterminate';
  importance: 'required' | 'recommended';
  explanation: string;
  affectedCount?: number;
}
```

`toId` may be absent when a required item cannot be resolved to a visible node. That renders as an open/broken socket with a named missing requirement. The UI must not invent a substitute node.

## 7. VisualHealthFinding

```ts
interface VisualHealthFinding {
  id: VisualId;
  severity: Severity;
  title: string;
  summary: string;
  affectedIds: VisualId[];
  suggestedAction?: string;
  structuredKind?: 'loader-compatibility' | 'content' | 'runtime' | 'recovery' | 'other';
  compatibility?: Compatibility;
  reviewIntent?: VisualIntent;
}
```

The adapter maps structured health data. It must never parse a human-readable backend message to infer severity, loader candidates, filenames, or an executable action. A recommendation remains non-blocking.

## 8. VisualLoaderCandidate

```ts
interface VisualLoaderCandidate {
  id: VisualId;
  family: string;
  version: string;
  channel: 'stable' | 'prerelease' | 'unknown';
  role: 'current' | 'recommended' | 'alternative';
  compatibility: Compatibility;
  requirementSummary: {
    satisfied: number;
    indeterminate: number;
    failed: number;
  };
  affectedContent: { visibleNames: string[]; total: number };
  explanation: string;
}
```

An indeterminate candidate is never styled or announced as compatible. A recommended candidate means the existing compatibility service can prove the recommendation; the presentation layer does not rank versions itself.

## 9. VisualInstallPlan

```ts
interface VisualInstallPlan {
  id: VisualId;
  action: 'install' | 'update' | 'remove';
  targetName: string;
  currentSummary: string;
  proposedSummary: string;
  dependencyChanges: Array<{ name: string; change: 'add' | 'update' | 'remove' }>;
  conflicts: Array<{ title: string; blocking: boolean; summary: string }>;
  fileChangeSummary: { add: number; replace: number; remove: number };
  recovery: {
    willCreateReturnPoint: boolean;
    summary: string;
  };
  blockers: string[];
  warnings: string[];
  choicesRemaining: number;
  freshness: 'current' | 'refresh-required';
}
```

This model is a read-only review projection produced from an already resolved plan. It cannot be sent back to the backend. The existing install controller retains the true plan/fingerprint and re-resolves when choices change.

## 10. VisualSnapshot

```ts
interface VisualSnapshot {
  id: VisualId;
  label: string;
  createdAt: string;
  role: 'manual' | 'known-good' | 'current-known-good' | 'undo-restore' | 'automatic';
  sizeLabel: string;
  changeSummary?: { added: number; changed: number; removed: number };
  protects: Array<'mods' | 'config' | 'worlds' | 'other-instance-files'>;
  worldProtection: 'included' | 'not-included' | 'unknown';
  availability: Availability;
}
```

The world/save boundary is mandatory. “Snapshot available” must never imply that worlds are protected when the selected snapshot scope excludes them. Content-addressing and object counts are implementation details.

## 11. VisualCrashEvidence

```ts
interface VisualCrashEvidence {
  incidentLabel: string;
  evidenceSources: Array<{
    kind: 'crash-report' | 'log' | 'process-outcome' | 'health';
    state: Knowledge;
    summary: string;
  }>;
  hypotheses: Array<{
    id: VisualId;
    title: string;
    strength: 'low' | 'medium' | 'high';
    supportingClues: string[];
    contradictoryClues: string[];
    state: 'candidate' | 'testing' | 'less-likely' | 'inconclusive';
  }>;
  experiment: {
    phase: 'read-only' | 'proposed' | 'running' | 'awaiting-player-confirmation' | 'complete';
    summary?: string;
    recoveryReady: boolean;
  };
  privacyNote: string;
}
```

Strength labels are presentation summaries, not causal probabilities. A changed crash, a successful launch, or a user confirmation updates the hypothesis state; none alone proves a cause.

## 12. VisualRuntimeState

```ts
interface VisualRuntimeState {
  runtime: {
    currentLabel: string;
    requiredJavaMajor?: number;
    compatibility: Compatibility;
    managedByAgora: boolean;
  };
  memory: {
    mode: VisualValue<'automatic' | 'manual'>;
    currentMiB: number;
    proposedMiB?: number;
    recommendedMiB?: number;
    safeHeadroomLabel?: string;
    explanation: string;
  };
  garbageCollector: VisualValue<
    | { mode: 'automatic' }
    | { mode: 'manual'; label: string }
  >;
  availability: Availability;
}
```

The workbench should usually render friendly units and a recommended zone, not raw command-line flags. JVM paths, complete arguments, runtime download metadata, and internal heuristics are excluded.

## 13. VisualNetworkReadiness

```ts
interface VisualNetworkReadiness {
  instanceName: string;
  launchMode: 'delegated' | 'direct' | 'unknown';
  policy: 'normal' | 'restricted' | 'lockdown';
  overall: 'ready' | 'needs-attention' | 'checking' | 'unknown';
  checkedAt?: string;
  checks: Array<{
    id: VisualId;
    category: 'game-files' | 'loader' | 'content' | 'java' | 'sign-in-and-launch';
    label: string;
    state: 'ready' | 'missing' | 'blocked-by-policy' | 'unknown';
    summary: string;
    nextIntent?: VisualIntent;
  }>;
}
```

This model may be populated live only by a truthful, core-owned read-only readiness query. Registry availability is not a proxy for artifact readiness. `unknown` is required when Agora cannot verify a category without mutating or making a disallowed request.

## 14. VisualIntent

```ts
type VisualIntent =
  | { kind: 'select'; entityId: VisualId }
  | { kind: 'inspect-relationship'; relationshipId: VisualId }
  | { kind: 'propose-install'; contentId: VisualId }
  | { kind: 'propose-update'; contentId: VisualId }
  | { kind: 'propose-remove'; contentId: VisualId }
  | { kind: 'propose-enabled'; contentId: VisualId; enabled: boolean }
  | { kind: 'review-health' }
  | { kind: 'review-loader'; candidateId?: VisualId }
  | { kind: 'open-crash-doctor' }
  | { kind: 'preview-snapshot'; snapshotId: VisualId }
  | { kind: 'request-snapshot-restore'; snapshotId: VisualId }
  | { kind: 'propose-memory'; mode: 'automatic' | 'manual'; memoryMiB?: number }
  | { kind: 'review-offline-readiness' }
  | { kind: 'open-guide'; topicId: GuideTopicId }
  | { kind: 'open-standard'; destination: StandardDestination };
```

`GuideTopicId` is a feature-owned closed/validated set sourced from the IDs in `GUIDE_TOPICS`; the current Guide exposes IDs as strings, so the navigation adapter must validate them. `StandardDestination` aliases Agora’s existing typed `Destination`, not an arbitrary URL or command string.

For live scenes the host adds the scene’s `viewRevision` outside the component callback before sending the intent to the controller. An intent expresses what the player wants to review. It is never authorization to bypass the existing confirmation flow.

Lab consumes the same union in its reducer. A Lab intent can only change simulated scene state or navigate out of Lab after an explicit exit.

## 15. Adapter mapping and deliberate omissions

| Presentation model | Current live source | Adapter rule | Never expose |
|---|---|---|---|
| VisualInstance | instance rows/manifest plus canonical process state | Merge only stable identity and player-facing status; process controller wins for launch state. | paths, raw manifest, process handles |
| VisualContentNode/Relationship | installed content plus resolved dependency data | Normalize nodes by ID; preserve missing/indeterminate edges. | hashes, provider URLs, receipt schema |
| VisualHealthFinding | structured health report | Map severity and structured evidence; do not parse message text for behavior. | scan token, cache identity |
| VisualLoaderCandidate | loader compatibility report | Preserve proven versus indeterminate status and backend ranking. | catalog internals, raw dependency specifications |
| VisualInstallPlan | resolved install plan | Summarize for display; keep the authoritative plan inside `InstallFlow`. | fingerprint, state hash, revision token, file paths |
| VisualSnapshot | snapshot list/diff | Render role, scope, and world boundary; backend remains authority for restore. | object hashes, manifest internals |
| VisualCrashEvidence | Crash Doctor evidence/suspects/outcome | Convert evidence into hypotheses and bounded summaries. | full log/path by default, false causal score |
| VisualRuntimeState | Java inspection and memory recommendation | Keep current/recommended/proposed distinct and explanations player-facing. | full JVM arguments, runtime download internals |
| VisualNetworkReadiness | future core readiness query | Preserve `unknown`; never infer missing categories. | cache paths, receipts, a fabricated guarantee |

## 16. Change control

Ordinary additions to lesson scenario data do not require domain changes. A request to add backend DTO fields directly to these models, execute from a component, weaken source discrimination, or remove uncertainty/current-proposed states is an architecture change and must return to Sol.
