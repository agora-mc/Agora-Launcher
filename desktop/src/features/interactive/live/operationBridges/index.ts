/**
 * Live operation bridges (SOL-2 BLOCKER B / §17.4).
 *
 * A bridge is a NARROW typed adapter from a live review route into the
 * existing STANDARD controller for the approved seam. Bridges never invoke a
 * mutation command themselves — they hand minimal typed context to the host of
 * the Standard surface, which performs its own fresh read/review. No bridge
 * exists for a rejected seam.
 *
 * `LiveReviewRoute` is a DISCRIMINATED union: `bridge` and `context.kind` are
 * locked together by construction, and `openBridge` verifies each arm's
 * context before dispatching. The install-flow context retains the requested
 * ACTION (`install` / `update` / `remove` / `review`) plus the content
 * reference, so the Standard surface can construct an action-bearing
 * `InstallIntent` instead of discarding the gesture.
 */

import type { ContentKind } from '../../domain/models';
import type { ApprovedBridge } from '../intentController';

/** The action a live install-flow gesture requests (never inferred from an id). */
export type InstallFlowAction = 'install' | 'update' | 'remove' | 'review';

export type BridgeContext =
  | { kind: 'health-review'; instanceId: string }
  | { kind: 'loader-review'; instanceId: string; candidateId?: string }
  | { kind: 'snapshot-compare'; instanceId: string; snapshotId: string }
  | { kind: 'crash-doctor'; instanceId: string }
  | {
      kind: 'install-flow';
      instanceId: string;
      action: InstallFlowAction;
      /** Visual content node id (never parsed for authority). */
      contentId?: string;
      /** Backend-derived filename resolved from the accepted live scene. */
      filename?: string;
      /** Backend-derived content kind resolved from the accepted live scene. */
      contentKind?: ContentKind;
    };

/** Discriminated route: `bridge` and `context.kind` can never disagree. */
export type LiveReviewRoute =
  | { bridge: 'health-review'; context: Extract<BridgeContext, { kind: 'health-review' }> }
  | { bridge: 'loader-review'; context: Extract<BridgeContext, { kind: 'loader-review' }> }
  | { bridge: 'snapshot-compare'; context: Extract<BridgeContext, { kind: 'snapshot-compare' }> }
  | { bridge: 'crash-doctor'; context: Extract<BridgeContext, { kind: 'crash-doctor' }> }
  | { bridge: 'install-flow'; context: Extract<BridgeContext, { kind: 'install-flow' }> };

export interface InstallFlowHandlerContext {
  instanceId: string;
  action: InstallFlowAction;
  contentId?: string;
  filename?: string;
}

/** The Standard surface the host must open for each approved bridge. */
export interface StandardBridgeHandlers {
  /** Fresh `checkInstanceHealth` then open the app-level `HealthDialog`. */
  openHealthReview(ctx: { instanceId: string; instanceName: string }): void;
  /** Fresh `planLoaderChange` then open `LoaderChooser`. */
  openLoaderReview(ctx: { instanceId: string }): void;
  /** Re-list the snapshot + `detectDrift`, then show the diff. */
  openSnapshotCompare(ctx: { instanceId: string; snapshotId: string }): void;
  /** Open the standard `CrashInvestigator` (navigation only). */
  openCrashDoctor(ctx: { instanceId: string }): void;
  /** Construct an action-bearing `InstallIntent` and open the canonical `InstallFlow`. */
  openInstallFlow(ctx: InstallFlowHandlerContext): void;
}

/**
 * Dispatch an approved review route to the matching Standard surface.
 * Returns false if the bridge/context is unknown (must never happen for a
 * route produced by the intent controller).
 */
export function openBridge(route: LiveReviewRoute, handlers: StandardBridgeHandlers): boolean {
  switch (route.bridge) {
    case 'health-review': {
      const ctx = route.context;
      handlers.openHealthReview({ instanceId: ctx.instanceId, instanceName: ctx.instanceId });
      return true;
    }
    case 'loader-review': {
      const ctx = route.context;
      handlers.openLoaderReview({ instanceId: ctx.instanceId });
      return true;
    }
    case 'snapshot-compare': {
      const ctx = route.context;
      handlers.openSnapshotCompare({ instanceId: ctx.instanceId, snapshotId: ctx.snapshotId });
      return true;
    }
    case 'crash-doctor': {
      const ctx = route.context;
      handlers.openCrashDoctor({ instanceId: ctx.instanceId });
      return true;
    }
    case 'install-flow': {
      const ctx = route.context;
      handlers.openInstallFlow({
        instanceId: ctx.instanceId,
        action: ctx.action,
        ...(ctx.contentId ? { contentId: ctx.contentId } : {}),
        ...(ctx.filename ? { filename: ctx.filename } : {}),
      });
      return true;
    }
    default:
      return false;
  }
}

/** Human label for an install-flow action (used for the review proposal). */
export function installActionLabel(action: InstallFlowAction): string {
  switch (action) {
    case 'install':
      return 'Install';
    case 'update':
      return 'Update';
    case 'remove':
      return 'Remove';
    case 'review':
      return 'Review staged changes';
  }
}

export type { ApprovedBridge };
