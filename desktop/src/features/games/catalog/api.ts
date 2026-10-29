/**
 * IPC wrappers for the catalog slice (the curated catalog for games other than Minecraft).
 * Mirrors `desktop/src-tauri/src/games/catalog.rs`. A reply that is not the documented shape
 * rejects, so a failure is never shown as an empty catalog.
 */

import { invoke } from '@tauri-apps/api/core';

/** Where a differing hash expectation came from (`HashOrigin` in agora-core). */
export type HashOrigin = 'curator_pin' | 'previous_install';

export interface CatalogRequirement {
  framework: string;
  minVersion: string | null;
}

export interface CatalogCompatibility {
  stores: string[];
  gameVersions: string[];
  requires: CatalogRequirement[];
  asset: string | null;
}

export interface CatalogEntry {
  id: string;
  name: string;
  author: string | null;
  description: string | null;
  /** `github_release` or `direct_hash`. */
  downloadStrategy: string;
  status: string;
  pageUrl: string | null;
  compatibility: CatalogCompatibility[];
}

export interface InstanceChoice {
  instanceId: string;
  name: string;
}

export interface CatalogListing {
  entries: CatalogEntry[];
  instances: InstanceChoice[];
}

export interface CatalogFramework {
  framework: string;
  name: string;
  minVersion: string | null;
  foundVersion: string | null;
}

export interface CatalogHash {
  sha256: string;
  verified: boolean;
  basis: string;
  confirmedPast: HashOrigin | null;
}

/** The plan fields every outcome carries. */
export interface CatalogPlan {
  itemId: string;
  itemName: string;
  instanceId: string;
  source: string;
  fileName: string;
  release: string | null;
  size: number | null;
  frameworks: CatalogFramework[];
  warnings: string[];
}

export type CatalogInstallReply =
  | ({ state: 'dry_run' } & CatalogPlan)
  | ({ state: 'already_installed'; installedRelease: string | null; sameRelease: boolean } & CatalogPlan)
  | ({ state: 'placed'; contentItemId: string; mountPath: string; size: number; hash: CatalogHash } & CatalogPlan)
  | ({
      state: 'needs_installer';
      contentItemId: string;
      size: number;
      hash: CatalogHash;
      reason: string;
      topLevel: string[];
    } & CatalogPlan);

export interface CatalogInstallRequest {
  gameId: string;
  instanceId: string;
  entryId: string;
  installAnyway: boolean;
  dryRun: boolean;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isCatalogEntry(value: unknown): value is CatalogEntry {
  return isRecord(value)
    && typeof value.id === 'string'
    && typeof value.name === 'string'
    && Array.isArray(value.compatibility);
}

function isInstanceChoice(value: unknown): value is InstanceChoice {
  return isRecord(value) && typeof value.instanceId === 'string' && typeof value.name === 'string';
}

/** The curated entries for a game, and the instances they can be installed into. */
export async function listCatalogEntries(gameId: string): Promise<CatalogListing> {
  const reply: unknown = await invoke<unknown>('games_catalog_list', { gameId });
  if (
    !isRecord(reply)
    || !Array.isArray(reply.entries)
    || !reply.entries.every(isCatalogEntry)
    || !Array.isArray(reply.instances)
    || !reply.instances.every(isInstanceChoice)
  ) {
    throw new Error('games_catalog_list did not return a catalog.');
  }
  return reply as unknown as CatalogListing;
}

/** Plan (`dryRun`) or install one entry into an instance. */
export async function installCatalogEntry(request: CatalogInstallRequest): Promise<CatalogInstallReply> {
  const reply: unknown = await invoke<unknown>('games_catalog_install', { request });
  if (!isRecord(reply) || typeof reply.state !== 'string' || typeof reply.itemId !== 'string') {
    throw new Error('games_catalog_install did not return an install result.');
  }
  return reply as unknown as CatalogInstallReply;
}
