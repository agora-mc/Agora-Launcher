import type { DatapackWorldStatus } from './tauri';

/**
 * Minecraft only loads data packs from inside a world, so Agora copies enabled
 * data packs into the instance's worlds. This is the one-line state shown for
 * a data pack row.
 */
export function datapackWorldLabel(status: DatapackWorldStatus | null | undefined, enabled: boolean): string | null {
  if (!status) return null;
  if (!enabled) return 'Not in any world while disabled';
  const total = status.available_worlds.length;
  if (total === 0) return 'No worlds yet — added to new worlds from their second session';
  if (status.all_worlds) return `All worlds (${total})`;
  if (status.covered_worlds === 0) return 'No worlds selected';
  if (status.covered_worlds >= total) return `All worlds (${total})`;
  return `${status.covered_worlds} of ${total} worlds`;
}

export const DATAPACK_SYNC_EXPLAINER =
  'Minecraft only loads data packs from inside each world. Agora copies the enabled ones into your worlds before every launch and whenever you change them. A world created during a session gets them at the next sync, so for a brand-new world that is its second session.';

/** Install-review line for a data pack: how many worlds it will reach, and the new-world caveat. */
export function datapackInstallNote(worldCount: number): string {
  const reach = worldCount === 0
    ? 'This instance has no worlds yet.'
    : `Will be added to all ${worldCount} world${worldCount === 1 ? '' : 's'} in this instance.`;
  return `${reach} Worlds created later get it from their second session (or use Sync now).`;
}
