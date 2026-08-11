/**
 * Source, capability, and freshness guards for interactive scenes.
 *
 * Sol-0 contract: `docs/interactive/SAFETY_BOUNDARIES.md` §4 (the live intent
 * gate). The first three gates are pure and live here: source, capability,
 * freshness. The remaining gates (availability, review, backend, outcome,
 * refresh) are owned by the live intent controller in `live/`.
 *
 * This module is pure: it imports nothing from React, Tauri, or the app layer.
 */

import type { CapabilityFlags, ExperienceSource, VisualScene } from './models';
import type { VisualIntent } from './intents';

export type GateResult =
  | { ok: true }
  | {
      ok: false;
      reason: 'simulation-source' | 'mixed-source' | 'missing-source' | 'capability' | 'refreshing' | 'stale' | 'unknown';
    };

export function isSimulationSource(source: ExperienceSource): boolean {
  return source.kind === 'simulation';
}

export function isLiveSource(source: ExperienceSource): boolean {
  return source.kind === 'live';
}

/** Source gate: a live intent must come from a live, unambiguous scene. */
export function sourceGate(source: ExperienceSource | undefined): GateResult {
  if (!source) return { ok: false, reason: 'missing-source' };
  if (source.kind === 'simulation') return { ok: false, reason: 'simulation-source' };
  return { ok: true };
}

/** Capability gate: the host enables only actions with an approved bridge. */
export function capabilityGate(intent: VisualIntent, capabilities: CapabilityFlags): GateResult {
  const enabled = capabilityFor(intent.kind, capabilities);
  return enabled ? { ok: true } : { ok: false, reason: 'capability' };
}

function capabilityFor(kind: VisualIntent['kind'], capabilities: CapabilityFlags): boolean {
  switch (kind) {
    case 'select':
    case 'inspect-relationship':
    case 'open-guide':
    case 'open-standard':
      return true; // navigation/exploration is always available
    case 'propose-install':
      return capabilities.canProposeInstall;
    case 'review-staged-changes':
      // Staged-change review routes to the install-flow bridge: gated by the
      // same install capability so default (all-off) stays non-executable.
      return capabilities.canProposeInstall;
    case 'propose-update':
      return capabilities.canProposeUpdate;
    case 'propose-remove':
      return capabilities.canProposeRemove;
    case 'propose-enabled':
      return capabilities.canProposeEnabled;
    case 'review-health':
      return capabilities.canReviewHealth;
    case 'review-loader':
      return capabilities.canReviewLoader;
    case 'open-crash-doctor':
      return capabilities.canOpenCrashDoctor;
    case 'preview-snapshot':
      return capabilities.canPreviewSnapshot;
    case 'request-snapshot-restore':
      return capabilities.canRequestSnapshotRestore;
    case 'propose-memory':
      return capabilities.canProposeMemory;
    case 'review-offline-readiness':
      return capabilities.canReviewOfflineReadiness;
    default:
      return false;
  }
}

/**
 * Freshness gate: a live intent carries a view revision. A scene that is
 * refreshing must WAIT for (or trigger) the mandatory re-read before any
 * review flow — 'refreshing' is never executable (FIX BEFORE LIVE MODE 1).
 */
export function freshnessGate(source: ExperienceSource): GateResult {
  if (source.kind !== 'live') return { ok: true }; // simulation scenes are controlled by the reducer
  switch (source.freshness) {
    case 'fresh':
      return { ok: true };
    case 'refreshing':
      return { ok: false, reason: 'refreshing' };
    case 'stale':
      return { ok: false, reason: 'stale' };
    case 'unknown':
      return { ok: false, reason: 'unknown' };
  }
}

/**
 * Combined pre-review gate for a live scene. This is the pure core of gates
 * 1-3; the live controller adds availability/review/backend/outcome/refresh.
 */
export function gateLiveIntent(
  scene: VisualScene | undefined,
  intent: VisualIntent,
  capabilities: CapabilityFlags,
): GateResult {
  if (!scene) return { ok: false, reason: 'missing-source' };
  const source = sourceGate(scene.source);
  if (!source.ok) return source;
  const capability = capabilityGate(intent, capabilities);
  if (!capability.ok) return capability;
  return freshnessGate(scene.source);
}

/**
 * True when an intent is permitted by the host's capability flags. Shared
 * visuals use this to hide/disable operation-shaped commands (SOL-2 BLOCKER 3);
 * the live controller enforces the same independently.
 */
export function isIntentEnabled(intent: VisualIntent, capabilities: CapabilityFlags): boolean {
  return capabilityGate(intent, capabilities).ok;
}
