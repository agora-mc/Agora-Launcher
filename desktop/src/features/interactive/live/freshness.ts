/**
 * Live freshness and revision helpers.
 *
 * A live scene carries a local view revision and observation time. Only a
 * `fresh` scene is executable: `refreshing`, `stale`, and `unknown` must wait
 * for (or trigger) the mandatory re-read before any review flow
 * (FIX BEFORE LIVE MODE 1). The revision is a local read-set identifier, never
 * a backend plan fingerprint.
 */

import type { ExperienceSource } from '../domain/models';

export type LiveFreshness = 'fresh' | 'refreshing' | 'stale' | 'unknown';

export function liveSource(viewRevision: string, freshness: LiveFreshness, observedAt?: string): ExperienceSource {
  return { kind: 'live', viewRevision, observedAt: observedAt ?? new Date().toISOString(), freshness };
}

/** Generate a local read-set revision. */
export function nextRevision(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

/** Fail-closed: only a fresh live scene is executable. */
export function isExecutable(source: ExperienceSource): boolean {
  return source.kind === 'live' && source.freshness === 'fresh';
}

/** A scene that is not fresh requires a re-read before any review flow. */
export function requiresRefresh(source: ExperienceSource): boolean {
  return source.kind === 'live' && source.freshness !== 'fresh';
}
