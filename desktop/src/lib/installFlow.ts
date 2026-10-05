// ---------------------------------------------------------------------------
// TypeScript types for the Install Pipeline (C0–C3)
// 1:1 mapping with agora-core::install_pipeline types.
// ---------------------------------------------------------------------------

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { UnlistenFn } from '@tauri-apps/api/event';

// --- Protocols ---

export type InstallAction =
  | { type: 'install'; sourceType: SourceType; itemId: string; candidateVersion?: string }
  | { type: 'update'; itemId: string; targetVersion: string }
  | { type: 'remove'; filename: string }
  | { type: 'batch-remove'; filenames: string[] }
  | { type: 'batch-update'; items: BatchUpdateItem[] }
  | { type: 'batch-install'; items: BatchInstallItem[] }
  | { type: 'repair-lockfile'; contentHash: string };

export interface BatchUpdateItem { itemId: string; targetVersion: string; }
/** `contentType` is absent for a mod; resource packs, shaders and data packs say so, which picks their folder. */
export interface BatchInstallItem { sourceType: SourceType; itemId: string; candidateVersion?: string; contentType?: string; }

/**
 * `provider` is any content provider — Agora's official ones or a plugin's.
 * Its item id is `provider:<provider-id>:<project-id>`.
 */
export type SourceType = 'curated' | 'modrinth' | 'manual' | 'provider';
export type OptionalDepsPolicy = { type: 'include'; deps: string[] }
  | { type: 'exclude-all' }
  | { type: 'prompt' };
export type RequestSource = 'interactive' | 'cli' | 'auto-update';

export interface PlanOverrides {
  allowReplace: boolean;
  skipHealthScan: boolean;
  allowClosestVersion?: boolean;
  /**
   * What a raw Modrinth install is (`resourcepack`, `shader`, `datapack`).
   * Absent means a mod. Decides which Modrinth loader tags are acceptable and
   * which folder the file installs into.
   */
  contentType?: string;
  skipItems?: string[];
  forceConflictResolution: Record<string, string>;
  /**
   * Explicit approval to switch the instance loader to this exact recommended
   * version so the incoming mods' loader requirements are satisfied. Only the
   * recommended version from the signed catalog is accepted; any other value
   * leaves the `loader-change` pending choice unresolved.
   */
  approveLoaderVersion?: string;
}

export interface InstallIntent {
  action: InstallAction;
  targetInstance: string;
  optionalDeps: OptionalDepsPolicy;
  requestedBy: RequestSource;
  overrides: PlanOverrides;
}

// --- Plan ---

export interface ResolvedInstallPlan {
  fingerprint: string;
  intent: InstallIntent;
  operation: ResolvedOperation;
  dependencies: ResolvedDep[];
  conflicts: DepConflict[];
  filesToAdd: FileAdd[];
  filesToRemove: FileRemove[];
  filesToDisable: FileDisable[];
  snapshot: SnapshotPlan;
  diskEstimate: DiskSpaceEstimate;
  warnings: PlanWarning[];
  blockingErrors: PlanError[];
  pendingChoices: PendingChoice[];
  /** Approved loader version switch, committed atomically with the file changes. */
  loaderChange?: LoaderChangePlan;
  createdAt: string;
  instanceStateHash: string;
  registryRevision: string;
}

/**
 * What a plan does, in words, for progress and completion messages. The same
 * background task runs installs, updates, removals and lockfile repairs, so
 * its labels come from the plan rather than from "installing".
 */
export function describePlanAction(plan: ResolvedInstallPlan): {
  /** "Removing 7 files", "Updating 3 items", "Installing 2 files". */
  verb: string;
  /** "Removed 7 files.", for the completion message. */
  done: string;
  /** Whether per-file download progress applies. */
  downloadsFiles: boolean;
} {
  const count = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;
  const added = plan.filesToAdd.length;
  const removed = plan.filesToRemove.length;
  const op = plan.operation.type;
  if (op === 'remove' || op === 'batch-remove') {
    const n = Math.max(removed, 1);
    return { verb: `Removing ${count(n, 'file')}`, done: `Removed ${count(n, 'file')}.`, downloadsFiles: false };
  }
  if (op === 'update' || op === 'batch-update') {
    const n = op === 'batch-update' && plan.operation.type === 'batch-update' ? plan.operation.operations.length : 1;
    return { verb: `Updating ${count(n, 'item')}`, done: `Updated ${count(n, 'item')}.`, downloadsFiles: added > 0 };
  }
  if (op === 'reconcile') {
    return { verb: 'Repairing to match the lockfile', done: 'Repair complete.', downloadsFiles: added > 0 };
  }
  const n = Math.max(added, 1);
  return { verb: `Installing ${count(n, 'file')}`, done: `Installed ${count(n, 'file')}.`, downloadsFiles: added > 0 };
}

/**
 * Clean plans can be applied without an interactive review. Anything that
 * adds a dependency, reports a warning/error, presents a conflict, or
 * changes an existing file stays in the focused review flow.
 */
export function planNeedsUserReview(
  plan: ResolvedInstallPlan,
  options: { ignoreDependencies?: boolean } = {},
): boolean {
  return plan.blockingErrors.length > 0
    || plan.pendingChoices.length > 0
    || plan.warnings.length > 0
    || plan.conflicts.length > 0
    || plan.filesToRemove.length > 0
    || plan.filesToDisable.length > 0
    || plan.dependencies.some((dependency) =>
      dependency.disposition.type === 'unresolved'
      || (!options.ignoreDependencies && dependency.disposition.type === 'install-candidate'),
    );
}

export type ResolvedOperation =
  | { type: 'install'; artifact: ResolvedArtifact }
  | { type: 'update'; oldVersionId: string; newArtifact: ResolvedArtifact }
  | { type: 'remove'; targetFilename: string; reverseDependents: ReverseDepInfo[] }
  | { type: 'batch-remove'; operations: ResolvedOperation[] }
  | { type: 'batch-update'; operations: ResolvedOperation[] }
  | { type: 'batch-install'; operations: ResolvedOperation[] }
  | { type: 'reconcile'; operations: ResolvedOperation[] };

export type ResolvedArtifact =
  | { type: 'download'; itemId: string; versionId: string; source: ArtifactSource; hashes: HashSpec; size: number; filename: string; metadata: ArtifactMetadata }
  | { type: 'local-file'; itemId: string; sourcePath: string; hashes: HashSpec; size: number; filename: string; metadata: ArtifactMetadata };

export interface ArtifactMetadata {
  sourceType: SourceType;
  registryId: string | null;
  modrinthId: string | null;
  contentType: string;
  version?: string | null;
}

export type ArtifactSource = { type: 'download'; url: string } | { type: 'local-file'; path: string };

export interface HashSpec { values: HashedValue[]; }
export interface HashedValue { algorithm: HashAlgorithm; value: string; }
export type HashAlgorithm = 'sha256' | 'sha512' | 'sha1';

export interface ResolvedDep {
  modJarId: string;
  requirement: 'required' | 'optional';
  source: 'jar' | 'manifest';
  disposition: DepDisposition;
  /** Human-readable project name when known; falls back to modJarId. */
  displayName?: string | null;
  /** Canonical upstream page URL when known. */
  pageUrl?: string | null;
  /** Selected batch items that pulled this dependency in. */
  requestedBy?: string[];
}

export type DepDisposition =
  | { type: 'reuse-existing'; modJarId: string; installedFilename: string }
  | { type: 'install-candidate'; artifact: ResolvedArtifact }
  | { type: 'included-in-batch'; targetFilename: string }
  | { type: 'excluded' }
  | { type: 'unresolved'; reason: string };

export interface DepConflict {
  conflictId: string;
  kind: ConflictKind;
  existingModJarId: string;
  incomingModJarId: string;
  message: string;
  blocking: boolean;
  resolutionOptions: ConflictResolution[];
  chosen?: ConflictResolution;
}

export type ConflictKind = 'version-conflict' | 'duplicate-mod' | 'loader-mismatch' | 'game-version-mismatch' | 'incompatible-mod' | 'broken-reverse-dep';
export type ConflictResolution = 'replace' | 'skip' | 'disable-existing' | 'abort' | 'remove-anyway';

export interface FileAdd { targetFilename: string; stagingFilename: string; artifact: ResolvedArtifact; hashes: HashSpec; size: number; }
export interface FileRemove { filename: string; }
export interface FileDisable { filename: string; }

export interface SnapshotPlan { label: string; estimatedBytes: number; }
export interface DiskSpaceEstimate { downloadBytes: number; snapshotBytes: number; applyOverheadBytes: number; peakAdditionalBytes: number; postCommitDeltaBytes: number; }

export interface PlanWarning { code: string; message: string; }
export interface PlanError { code: string; message: string; }

export type PendingChoice =
  | { type: 'optional-dependencies'; choiceId: string; options: OptionalDepOption[] }
  | { type: 'conflict'; choiceId: string; conflictId: string; options: ConflictResolutionOption[] }
  | {
      type: 'loader-change';
      choiceId: string;
      loader: string;
      currentVersion: string;
      recommendedVersion: string;
      compatibleVersions: string[];
      requirements: LoaderRequirementIssue[];
      conflicts: LoaderConflict[];
    };

export interface OptionalDepOption { modJarId: string; displayName: string; }
export interface ConflictResolutionOption { resolution: string; label: string; description: string; }

export interface LoaderChangePlan { loader: string; fromVersion: string; toVersion: string; }

/**
 * Mirrors `agora_core::health::LoaderRequirementIssue` (snake_case JSON keys,
 * identical to the payload health blockers attach to loader findings).
 */
export interface LoaderRequirementIssue {
  declaring_mod_id: string | null;
  declaring_mod_ids?: string[];
  target_id: string;
  version_ranges: string[];
  importance: 'required' | 'recommended' | 'suggested';
  candidate_version: string | null;
  verdict: LoaderRequirementVerdict;
}
export type LoaderRequirementVerdict =
  | 'satisfied'
  | 'unsatisfied'
  | { unsupported: { reason: string } };

/** Mirrors `agora_core::loader_compatibility::LoaderConflict`. */
export interface LoaderConflict {
  declaring_mod_id: string | null;
  target_id: string;
  version_ranges: string[];
  with_declaring_mod_id: string | null;
  with_target_id: string;
  with_version_ranges: string[];
  message: string;
}

export interface ReverseDepInfo { modJarId: string; filename: string; requirement: string; impact?: string; }

// --- Progress ---

export interface ProgressEvent {
  planId: string;
  phase: ProgressPhase;
  step: number;
  totalSteps: number;
  bytesDownloaded: number;
  bytesTotal: number;
  message: string;
}

export type ProgressPhase = 'resolving' | 'staging' | 'snapshotting' | 'applying' | 'health-scan' | 'done' | 'failed' | 'cancelled';

// --- Outcome ---

// Minimal health shape for the install outcome — the full HealthReport type
// lives in tauri.ts; this keeps the module cycle-free while staying typed
// enough for the corner UI to render blocker counts and messages.
export interface HealthBlocker { kind: string; mod_id: string | null; filename: string | null; message: string; suggested_action: string | null; }
export interface HealthWarning { kind: string; mod_id: string | null; filename: string | null; message: string; suggested_action: string | null; }
export interface HealthReportLike { score: 'green' | 'yellow' | 'red'; warnings: HealthWarning[]; blockers: HealthBlocker[]; recommendations: unknown[]; scan_token: string; }

export type InstallOutcome =
  | { type: 'success'; installedItems: string[]; existingItemsReused: string[]; warnings: PlanWarning[]; health: HealthOutcome; snapshotId: string }
  // Health blockers were found after applying. The install is **kept** so the
  // user can inspect and repair; snapshotId is the pre-install recovery
  // snapshot for manual rollback if desired.
  | { type: 'health-rollback'; healthReport: HealthReportLike; snapshotId: string; warnings: PlanWarning[] }
  | { type: 'cancelled'; phase: string; rollbackPerformed: boolean }
  | { type: 'failed'; error: string; rollbackPerformed: boolean; snapshotId: string | null };

export type HealthOutcome =
  | { type: 'completed'; report: HealthReportLike }
  | { type: 'skipped'; reason: string };

// --- Cancellation Token ---

export class CancellationToken {
  private _cancelled = false;
  cancel() { this._cancelled = true; }
  get isCancelled() { return this._cancelled; }
}

// ---------------------------------------------------------------------------
// Tauri facades (thin command wrappers)
// ---------------------------------------------------------------------------

export const resolveInstallPlan = (intent: InstallIntent) =>
  invoke<ResolvedInstallPlan>('resolve_install_plan', { intent });

export const applyInstallPlan = (plan: ResolvedInstallPlan) =>
  invoke<InstallOutcome>('apply_install_plan', { planId: plan.fingerprint });

export const cancelInstall = (planId: string) =>
  invoke<void>('cancel_install', { planId });

/** Subscribe to progress events for a given plan. Returns an unsubscribe function. */
export function subscribeProgress(
  planId: string,
  handler: (event: ProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProgressEvent>('install:progress', (event) => {
    if (event.payload.planId === planId) {
      handler(event.payload);
    }
  });
}
