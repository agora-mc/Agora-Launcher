/**
 * Live intent controller (SOL-2 BLOCKER 2).
 *
 * Owns the pre-review gate sequence for every live intent:
 *
 *   source -> capability -> freshness -> entity/duplicate availability
 *          -> route to an APPROVED Standard review bridge
 *
 * Only SOL-2-approved seams map to a bridge; rejected seams (disable, restore,
 * memory, direct repair, direct crash experiments) are blocked by capability
 * and are never routed here. The controller NEVER invokes a mutation — it only
 * returns a review route (or a blocked/refresh decision) that the host turns
 * into opening the existing Standard surface.
 */

import { capabilityGate, sourceGate } from '../domain/guards';
import type { CapabilityFlags, VisualScene } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { hasProposalInFlight } from '../domain/state';
import { requiresRefresh } from './freshness';
import type { BridgeContext, LiveReviewRoute } from './operationBridges';

/** The SOL-2-approved Standard operation bridges. */
export type ApprovedBridge =
  | 'health-review'
  | 'loader-review'
  | 'snapshot-compare'
  | 'crash-doctor'
  | 'install-flow';

export type RouteResult =
  | { status: 'selection' }
  | { status: 'navigate'; intent: VisualIntent }
  | { status: 'review'; route: LiveReviewRoute; intent: VisualIntent }
  | { status: 'refresh-required' }
  | { status: 'blocked'; reason: string; gate: string };

/**
 * Pair an approved bridge with its context into the DISCRIMINATED
 * `LiveReviewRoute`. `bridgeForIntent` and `contextForBridge` are both derived
 * from the same intent, so `bridge` and `context.kind` always agree — the casts
 * below only recover that agreement for TypeScript.
 */
function reviewRouteFor(bridge: ApprovedBridge, context: BridgeContext): LiveReviewRoute {
  switch (bridge) {
    case 'health-review':
      return { bridge, context: context as Extract<BridgeContext, { kind: 'health-review' }> };
    case 'loader-review':
      return { bridge, context: context as Extract<BridgeContext, { kind: 'loader-review' }> };
    case 'snapshot-compare':
      return { bridge, context: context as Extract<BridgeContext, { kind: 'snapshot-compare' }> };
    case 'crash-doctor':
      return { bridge, context: context as Extract<BridgeContext, { kind: 'crash-doctor' }> };
    case 'install-flow':
      return { bridge, context: context as Extract<BridgeContext, { kind: 'install-flow' }> };
  }
}

/** Map an intent to its approved bridge (or null when unsupported/rejected). */
export function bridgeForIntent(intent: VisualIntent): ApprovedBridge | null {
  switch (intent.kind) {
    case 'review-health':
      return 'health-review';
    case 'review-loader':
      return 'loader-review';
    case 'preview-snapshot':
      return 'snapshot-compare';
    case 'open-crash-doctor':
      return 'crash-doctor';
    case 'review-staged-changes':
    case 'propose-install':
    case 'propose-update':
    case 'propose-remove':
      return 'install-flow';
    default:
      return null;
  }
}

/** The existing authoritative operation for each approved bridge (recorded, not invoked). */
export function operationSeamFor(bridge: ApprovedBridge): string {
  switch (bridge) {
    case 'health-review':
      return 'checkInstanceHealth scan -> HealthDialog (blocker override semantics)';
    case 'loader-review':
      return 'planLoaderChange -> LoaderChooser / loader change + fresh post-change health';
    case 'snapshot-compare':
      return 're-list snapshot -> detectDrift diff (read-only)';
    case 'crash-doctor':
      return 'CrashInvestigator (recovery-first experiment flow)';
    case 'install-flow':
      return 'InstallIntent -> InstallFlow -> canonical install pipeline';
  }
}

/** Minimal typed context for an approved bridge (SOL-2 BLOCKER B). */
export function contextForBridge(
  intent: VisualIntent,
  instanceId: string,
): BridgeContext | null {
  switch (intent.kind) {
    case 'review-health':
      return { kind: 'health-review', instanceId };
    case 'review-loader':
      return { kind: 'loader-review', instanceId, ...(intent.candidateId ? { candidateId: intent.candidateId } : {}) };
    case 'preview-snapshot':
      return { kind: 'snapshot-compare', instanceId, snapshotId: intent.snapshotId };
    case 'open-crash-doctor':
      return { kind: 'crash-doctor', instanceId };
    case 'propose-install':
      // Retain the requested action + content so the Standard surface can
      // construct an action-bearing InstallIntent (never inferred from an id).
      return { kind: 'install-flow', instanceId, action: 'install', contentId: intent.contentId };
    case 'propose-update':
      return { kind: 'install-flow', instanceId, action: 'update', contentId: intent.contentId };
    case 'propose-remove':
      return { kind: 'install-flow', instanceId, action: 'remove', contentId: intent.contentId };
    case 'review-staged-changes':
      return { kind: 'install-flow', instanceId, action: 'review' };
    default:
      return null;
  }
}

/**
 * Typed instance readiness/lock inputs for the availability gate (SOL-2 §18.3).
 * These are derived from the projected live scene + canonical app state and
 * are NOT collapsed into a single boolean so each condition can be explained.
 */
export interface AvailabilityInput {
  /** Player lock: the instance is `locked-by-player`. */
  locked: boolean;
  /** Recovery readiness is `preparing` or `failed` (mutation must wait). */
  recoveryBusy: boolean;
  /** A process/launch is active for the instance (running/starting/stopping/delegated or process-unknown). */
  processBusy: boolean;
  /** A canonical install transaction is active for the instance. */
  installBusy: boolean;
}

export function allAvailable(input: AvailabilityInput): boolean {
  return !input.locked && !input.recoveryBusy && !input.processBusy && !input.installBusy;
}

/**
 * Route a live intent through the gate sequence. `scene` may be undefined when
 * no live scene is available (missing source). Instance readiness/lock state
 * (from the projected live scene + app-level controllers) is honored by
 * `availability`.
 */
export function routeLiveIntent(
  scene: VisualScene | undefined,
  intent: VisualIntent,
  capabilities: CapabilityFlags,
  instanceId: string,
  availability: AvailabilityInput = { locked: false, recoveryBusy: false, processBusy: false, installBusy: false },
): RouteResult {
  if (intent.kind === 'select') return { status: 'selection' };
  if (intent.kind === 'inspect-relationship') return { status: 'selection' };
  if (intent.kind === 'open-guide' || intent.kind === 'open-standard') {
    return { status: 'navigate', intent };
  }

  const source = sourceGate(scene?.source);
  if (!source.ok) {
    return { status: 'blocked', reason: gateReason(source.reason), gate: source.reason };
  }

  const capability = capabilityGate(intent, capabilities);
  if (!capability.ok) {
    return { status: 'blocked', reason: gateReason(capability.reason), gate: capability.reason };
  }

  // Freshness re-read: a non-fresh live scene must be refreshed before any
  // review — refreshing/stale/unknown are never executable.
  if (scene && requiresRefresh(scene.source)) {
    return { status: 'refresh-required' };
  }

  // Availability gate: player locks, pending/failed recovery, active
  // process/launch, and active installs each block review with their own
  // explanation (SAFETY_BOUNDARIES gate + MASTER_SPEC §19.15). Selection and
  // inspection remain available above.
  if (availability.locked) {
    return { status: 'blocked', reason: 'This instance is locked by another player.', gate: 'availability' };
  }
  if (availability.recoveryBusy) {
    return { status: 'blocked', reason: 'Recovery is pending or failed for this instance — finish recovery before reviewing.', gate: 'availability' };
  }
  if (availability.processBusy) {
    return { status: 'blocked', reason: 'A process is active for this instance — wait for it to stop before reviewing.', gate: 'availability' };
  }
  if (availability.installBusy) {
    return { status: 'blocked', reason: 'An install is active for this instance — wait for it to finish before reviewing.', gate: 'availability' };
  }
  // Duplicate gate: an in-flight review blocks new reviews.
  if (scene && hasProposalInFlight(scene)) {
    return { status: 'blocked', reason: 'Another review is already in progress.', gate: 'availability' };
  }

  const bridge = bridgeForIntent(intent);
  const context = contextForBridge(intent, instanceId);
  if (!bridge || !context) {
    return {
      status: 'blocked',
      reason: 'This action is not available in the live view.',
      gate: 'unsupported',
    };
  }

  return { status: 'review', route: reviewRouteFor(bridge, context), intent };
}

function gateReason(reason: string): string {
  switch (reason) {
    case 'simulation-source':
      return 'Simulation scenes cannot drive live reviews.';
    case 'missing-source':
      return 'No live scene is available — refresh or use Standard view.';
    case 'capability':
      return 'This action is not enabled for this view.';
    case 'refreshing':
      return 'Waiting for the live refresh to complete.';
    case 'stale':
      return 'The live data is stale — refresh before reviewing.';
    case 'unknown':
      return 'Live data is incomplete — refresh before reviewing.';
    default:
      return 'This action is not available right now.';
  }
}
