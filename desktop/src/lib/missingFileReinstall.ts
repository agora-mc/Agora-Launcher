import type { InstallIntent } from './installFlow';
import type { InstalledMod } from './tauri';

/**
 * Where a manifest entry can be fetched again from, for putting a missing
 * file back at the version the instance already records.
 *
 * `null` means Agora has no source it can name, so the user has to find the
 * mod in Browse. Imported setups without catalog or Modrinth ids land here.
 */
export type ReinstallSource =
  | { kind: 'curated'; itemId: string; version: string }
  | { kind: 'modrinth'; projectId: string; version: string };

export function reinstallSourceFor(entry: InstalledMod): ReinstallSource | null {
  // The Health Check's missing-file finding only covers mod jars.
  if (entry.content_type !== 'mod') return null;
  const version = entry.version?.trim() ?? '';
  if (version === '') return null;
  // A curated registry id wins: the curated pipeline keeps the catalog's
  // dependency graph, the same choice Browse makes for a curated item.
  const registryId = entry.registry_id?.trim() ?? '';
  if (registryId !== '') return { kind: 'curated', itemId: registryId, version };
  const modrinthId = entry.modrinth_id?.trim() ?? '';
  if (modrinthId !== '') return { kind: 'modrinth', projectId: modrinthId, version };
  return null;
}

/**
 * The install intent Browse builds for the same reinstall. `candidateVersion`
 * is the Modrinth version id for a Modrinth source and the catalog version
 * string for a curated one.
 */
export function reinstallIntent(
  instanceId: string,
  sourceType: 'curated' | 'modrinth',
  itemId: string,
  candidateVersion: string,
): InstallIntent {
  return {
    action: { type: 'install', sourceType, itemId, candidateVersion },
    targetInstance: instanceId,
    optionalDeps: { type: 'prompt' },
    requestedBy: 'interactive',
    overrides: {
      allowReplace: false,
      skipHealthScan: false,
      forceConflictResolution: {},
    },
  };
}

/** A file name without its `.jar` extension, for a search query. */
export function searchQueryForFile(filename: string): string {
  return filename.replace(/\.jar(\.disabled)?$/i, '').trim() || filename;
}
