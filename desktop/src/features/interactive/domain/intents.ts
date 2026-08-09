/**
 * Closed `VisualIntent` union and navigation destination aliases.
 *
 * Sol-0 contract: `docs/interactive/DOMAIN_MODELS.md` §14. An intent expresses
 * what the player wants to review; it is never authorization to bypass the
 * existing confirmation flow. Lab consumes the same union in its reducer and
 * can only change simulated state or navigate out after explicit exit.
 *
 * This module is pure: it imports nothing from React, Tauri, or the app layer.
 */

import type { VisualId } from './models';

/**
 * Closed/validated set of Field Guide topic IDs, sourced from
 * `desktop/src/data/guideContent.ts` `GUIDE_TOPICS`. The Guide exposes IDs as
 * strings, so the navigation adapter must validate them against this set.
 */
export const GUIDE_TOPIC_IDS = [
  'getting-started',
  'modding-foundations',
  'home-navigation',
  'instances',
  'browse-registry',
  'install-update',
  'content-management',
  'launching',
  'crash-recovery',
  'snapshots-loadouts',
  'packs-sharing',
  'java-performance',
  'settings-appearance',
  'accounts-services',
  'privacy-offline',
  'governance',
  'ai-assistant',
  'mcp-automation',
] as const;

export type GuideTopicId = (typeof GUIDE_TOPIC_IDS)[number];

export function isGuideTopicId(value: unknown): value is GuideTopicId {
  return typeof value === 'string' && (GUIDE_TOPIC_IDS as readonly string[]).includes(value);
}

/**
 * Alias of Agora's existing typed `Destination` (`desktop/src/lib/useDestination.ts`).
 * Defined locally so `domain/` stays app-layer-free; the navigation adapter
 * validates and maps this onto the real `Destination`.
 */
export type StandardDestination =
  | { type: 'tab'; tab: 'home' | 'browse' | 'instances' | 'governance' | 'guide' | 'about' | 'settings' }
  | { type: 'mod-detail'; itemId: string; browseInstanceId?: string }
  | { type: 'instance-detail'; instanceId: string };

export function isStandardDestination(value: unknown): value is StandardDestination {
  if (!value || typeof value !== 'object') return false;
  const dest = value as Record<string, unknown>;
  if (dest.type === 'tab') {
    return typeof dest.tab === 'string'
      && ['home', 'browse', 'instances', 'governance', 'guide', 'about', 'settings'].includes(dest.tab);
  }
  if (dest.type === 'mod-detail') {
    return typeof dest.itemId === 'string'
      && (dest.browseInstanceId === undefined || typeof dest.browseInstanceId === 'string');
  }
  if (dest.type === 'instance-detail') {
    return typeof dest.instanceId === 'string';
  }
  return false;
}

/** Closed set of intents a player-facing visual can emit. */
export type VisualIntent =
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
  /** Narrow review intent: staged changes are ready for the existing review surface. */
  | { kind: 'review-staged-changes' }
  | { kind: 'open-guide'; topicId: GuideTopicId }
  | { kind: 'open-standard'; destination: StandardDestination };
