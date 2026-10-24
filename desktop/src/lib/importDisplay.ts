/**
 * Display rules for launcher imports (the wizard, the instance cards and the
 * instance editor). Kept pure so the wording and the numbers can be tested
 * without rendering a dialog.
 */

/**
 * Memory in one format everywhere (review, editor label, slider label):
 * 12064 MB reads as "11.8 GB", 12288 MB as "12 GB", 4096 MB as "4 GB".
 */
export function formatMemoryMb(mb: number): string {
  if (mb >= 1024) {
    const gb = mb / 1024;
    return `${(mb % 1024 === 0 ? gb : Number(gb.toFixed(1)))} GB`;
  }
  return `${mb} MB`;
}

const FILE_SOURCE_PREFIX = 'mrpack:';

/** Whether an instance came from a pack file on disk rather than another launcher. */
export function isFileImportSource(source: string | null | undefined): boolean {
  return (source ?? '').trim().toLowerCase().startsWith(FILE_SOURCE_PREFIX);
}

/**
 * Launcher names as users know them, from the snake_case kind or the source
 * label. An instance imported from a pack file is recorded as
 * `mrpack:<file name>` and shows that file name.
 */
export function importSourceLabel(source: string | null | undefined): string | null {
  if (isFileImportSource(source)) {
    const file = (source ?? '').trim().slice(FILE_SOURCE_PREFIX.length).trim();
    return file === '' ? 'a pack file' : file;
  }
  switch ((source ?? '').trim().toLowerCase()) {
    case 'curseforge':
    case 'curse_forge':
      return 'CurseForge';
    case 'modrinth':
      return 'Modrinth';
    case 'prism':
      return 'Prism Launcher';
    default:
      return null;
  }
}

interface ProgressLike {
  progress: number | null;
  bytesDownloaded: number | null;
  bytesTotal: number | null;
}

/**
 * Percent done for the import progress bar. Bytes win when the backend sent a
 * byte total; the fraction is the fallback. Clamped to 0..100 and never NaN.
 */
export function importProgressPercent(p: ProgressLike | null): number {
  if (!p) return 0;
  let fraction: number | null = null;
  if (p.bytesTotal != null && p.bytesTotal > 0 && p.bytesDownloaded != null) {
    fraction = p.bytesDownloaded / p.bytesTotal;
  } else if (p.progress != null) {
    fraction = p.progress;
  }
  if (fraction == null || !Number.isFinite(fraction)) return 0;
  return Math.round(Math.min(Math.max(fraction, 0), 1) * 100);
}

/** Memory provenance stored in the instance manifest at import time. */
export interface CopiedMemory {
  launcher: string;
  mb: number;
}

/**
 * The "Copied from CurseForge: 11.8 GB" line. It only shows while the value
 * in use is still the copied one and automatic memory is off; once the user
 * changes the number, the copy is no longer what they are looking at.
 */
export function copiedMemoryNote(
  copied: CopiedMemory | null,
  currentMb: number,
  mode: 'auto' | 'manual',
): string | null {
  if (!copied || mode !== 'manual') return null;
  if (!Number.isFinite(copied.mb) || copied.mb !== currentMb) return null;
  const launcher = importSourceLabel(copied.launcher) ?? 'the source launcher';
  return `Copied from ${launcher}: ${formatMemoryMb(copied.mb)}`;
}

/** Reads the `memory_copied` record out of an instance manifest's user_preferences. */
export function readCopiedMemory(userPreferences: unknown): CopiedMemory | null {
  if (!userPreferences || typeof userPreferences !== 'object') return null;
  const record = (userPreferences as { memory_copied?: unknown }).memory_copied;
  if (!record || typeof record !== 'object') return null;
  const { launcher, mb } = record as { launcher?: unknown; mb?: unknown };
  if (typeof launcher !== 'string' || typeof mb !== 'number' || !Number.isFinite(mb)) return null;
  return { launcher, mb };
}

/**
 * "12 mods · 3 resource packs · 1 shader pack · 2 worlds". A count the backend
 * could not read (null) says "count unknown" rather than a number that looks
 * complete.
 */
export function contentCountsLabel(counts: {
  mod_count: number | null | undefined;
  resourcepack_count: number | null | undefined;
  shaderpack_count: number | null | undefined;
  world_count: number | null | undefined;
}): string {
  const part = (count: number | null | undefined, one: string, many: string): string =>
    count == null ? `${many} count unknown` : `${count} ${count === 1 ? one : many}`;
  return [
    part(counts.mod_count, 'mod', 'mods'),
    part(counts.resourcepack_count, 'resource pack', 'resource packs'),
    part(counts.shaderpack_count, 'shader pack', 'shader packs'),
    part(counts.world_count, 'world', 'worlds'),
  ].join(' · ');
}

/** The name an outcome's instance id was planned under; falls back to the id. */
export function outcomeDisplayName(
  instanceId: string,
  items: ReadonlyArray<{ destination_id: string; destination_name: string }>,
): string {
  const match = items.find((item) => item.destination_id === instanceId);
  const name = match?.destination_name.trim();
  return name ? name : instanceId;
}
