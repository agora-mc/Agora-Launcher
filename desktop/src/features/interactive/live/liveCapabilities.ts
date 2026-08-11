/**
 * Capabilities for the High Interaction live surface (SOL-2 APPROVED §20).
 *
 * The Sol-approved seams are enabled: remove through InstallFlow, review-only
 * health inspection, loader plan/chooser/change, read-only snapshot compare,
 * and Crash Doctor navigation. Every rejected or still-blocked seam stays OFF:
 * dependency disable, install/update (blocked on a fresh curated
 * candidate/target-version source, §19.5), snapshot restore, direct health
 * repair, crash experiments, memory mutation, and offline readiness.
 */

import type { CapabilityFlags } from '../domain/models';

export function liveHighInteractionCapabilities(): CapabilityFlags {
  return {
    canProposeInstall: false, // BLOCKED: needs a fresh curated candidate source (§19.5)
    canProposeUpdate: false, // BLOCKED: needs a fresh curated candidate/target-version source (§19.5)
    canProposeRemove: true, // APPROVED: remove through the reviewed InstallFlow
    canProposeEnabled: false, // REJECTED: dependency-aware disable
    canReviewHealth: true, // APPROVED: review-only health inspection (Standard dialog)
    canReviewLoader: true, // APPROVED: loader plan/chooser/change (Standard seam)
    canOpenCrashDoctor: true, // APPROVED: Crash Doctor navigation only
    canPreviewSnapshot: true, // APPROVED: read-only detectDrift compare
    canRequestSnapshotRestore: false, // REJECTED: snapshot restore
    canProposeMemory: false, // REJECTED: memory mutation
    canReviewOfflineReadiness: false, // not approved
  };
}
