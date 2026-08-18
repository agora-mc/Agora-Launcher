/**
 * Interactive presentation domain models.
 *
 * These are presentation models, not persistence schemas, command payloads,
 * or copies of Rust/Tauri DTOs. Backend authority data (hashes, plan
 * fingerprints, scan tokens, receipts, paths, raw manifests) never appears
 * here.
 *
 * This module is pure: it imports nothing from React, Tauri, or the app layer.
 */

import type { VisualIntent } from './intents';

/** Stable opaque identity for focus and relationships. Never a file path. */
export type VisualId = string;

/** Explicit origin for every scene: simulation vs live. */
export type ExperienceSource =
  | { kind: 'simulation'; scenarioId: string; scenarioVersion: number }
  | {
      kind: 'live';
      viewRevision: string;
      observedAt: string;
      freshness: 'fresh' | 'refreshing' | 'stale' | 'unknown';
    };

export type Knowledge = 'known' | 'unknown' | 'unavailable';
export type Availability = 'available' | 'busy' | 'locked' | 'unavailable';
export type Severity = 'blocker' | 'warning' | 'recommendation';
export type Compatibility = 'compatible' | 'indeterminate' | 'incompatible' | 'unknown';

/** A value that may carry a local proposed alternative to current. */
export interface VisualValue<T> {
  current: T;
  proposed?: T;
}

/** Capability flags supplied by the host. Visuals never decide authority. */
export interface CapabilityFlags {
  canProposeInstall: boolean;
  canProposeUpdate: boolean;
  canProposeRemove: boolean;
  canProposeEnabled: boolean;
  canReviewHealth: boolean;
  canReviewLoader: boolean;
  canOpenCrashDoctor: boolean;
  canPreviewSnapshot: boolean;
  canRequestSnapshotRestore: boolean;
  canProposeMemory: boolean;
  canReviewOfflineReadiness: boolean;
}

export const NO_CAPABILITIES: CapabilityFlags = {
  canProposeInstall: false,
  canProposeUpdate: false,
  canProposeRemove: false,
  canProposeEnabled: false,
  canReviewHealth: false,
  canReviewLoader: false,
  canOpenCrashDoctor: false,
  canPreviewSnapshot: false,
  canRequestSnapshotRestore: false,
  canProposeMemory: false,
  canReviewOfflineReadiness: false,
};

/** The shared scene envelope rendered by controlled visuals. */
export interface VisualScene {
  source: ExperienceSource;
  instance?: VisualInstance;
  content: VisualContentNode[];
  relationships: VisualRelationship[];
  findings: VisualHealthFinding[];
  proposals: VisualProposal[];
}

/** Phase of a proposal. "Committed" is an outcome event, not an overlay. */
export type ProposalPhase = 'proposed' | 'in-review' | 'applying' | 'rejected';

export interface VisualProposal {
  id: VisualId;
  intent: VisualIntent;
  phase: ProposalPhase;
  title: string;
  summary: string;
  destructive: boolean;
}

export type InstanceLockState = 'editable' | 'locked-by-player' | 'busy';
export type RecoveryReadiness = 'ready' | 'preparing' | 'failed' | 'unknown';
export type LaunchState = 'idle' | 'starting' | 'running' | 'stopping' | 'delegated' | 'failed';

export interface VisualInstance {
  id: VisualId;
  name: string;
  gameVersion: string;
  loader: VisualValue<{
    family: string;
    version?: string;
    compatibility: Compatibility;
  }>;
  lockState: InstanceLockState;
  recoveryReadiness: RecoveryReadiness;
  launchState: LaunchState;
  contentSummary: {
    enabled: number;
    disabled: number;
    needsAttention: number;
  };
}

export type ContentKind = 'mod' | 'modpack' | 'resource-pack' | 'shader' | 'datapack' | 'world';

export interface VisualContentNode {
  id: VisualId;
  name: string;
  /**
   * Exact on-disk filename, present only when `name` is a DERIVED label.
   * Keeps the derivation honest: the authoritative identifier stays visible
   * next to the friendly one (SOL §22.5).
   */
  fileLabel?: string;
  kind: ContentKind;
  /** Public HTTPS icon URL when installed-content metadata resolved one. */
  iconUrl?: string;
  version?: VisualValue<string | null>;
  presence: VisualValue<'installed' | 'not-installed'>;
  enabled: VisualValue<boolean>;
  health: 'healthy' | 'needs-attention' | 'blocked' | 'unknown';
  /**
   * Public catalogue identities, when known. Not private authority data — these
   * are the ids the reviewed Browse surface already shows — and they are what
   * lets the detail panel look up a description without a per-mod fetch for the
   * whole pack.
   */
  catalogIds?: { registryId: string | null; modrinthId: string | null; modJarId?: string | null };
  relationshipSummary: {
    requiredBy: number;
    requires: number;
    conflicts: number;
  };
  availability: Availability;
}

export type RelationshipKind = 'requires' | 'recommends' | 'conflicts-with';
export type RelationshipState = 'satisfied' | 'missing' | 'conflicting' | 'indeterminate';
export type RelationshipImportance = 'required' | 'recommended';

export interface VisualRelationship {
  id: VisualId;
  fromId: VisualId;
  /** Absent when a required item cannot be resolved to a visible node. */
  toId?: VisualId;
  kind: RelationshipKind;
  state: RelationshipState;
  importance: RelationshipImportance;
  explanation: string;
  affectedCount?: number;
}

export interface VisualHealthFinding {
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

export interface VisualLoaderCandidate {
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

export interface VisualInstallPlan {
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

export interface VisualSnapshot {
  id: VisualId;
  label: string;
  /** Display label for the creation time (e.g. "Today, 09:00"). */
  createdAt: string;
  /** Authoritative sort key (ISO/epoch) — separate from the display label so
   * ordering is chronological even when labels are localized (FIX BEFORE LIVE
   * MODE 4). */
  sortKey?: string;
  role: 'manual' | 'known-good' | 'current-known-good' | 'undo-restore' | 'automatic';
  sizeLabel: string;
  changeSummary?: { added: number; changed: number; removed: number };
  protects: Array<'mods' | 'config' | 'worlds' | 'other-instance-files'>;
  worldProtection: 'included' | 'not-included' | 'unknown';
  availability: Availability;
}

export interface VisualCrashEvidence {
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

export interface VisualRuntimeState {
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

export interface VisualNetworkReadiness {
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
