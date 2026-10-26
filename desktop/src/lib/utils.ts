import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatDate(value: string | null | undefined): string {
  if (!value) return 'Unknown';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return 'Unknown';
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  }).format(date);
}

/**
 * A stored UTC timestamp as the user's local date and time. Raw values such as
 * `2026-10-22T18:15:58.781044+00:00` read as machine output, not as a time.
 */
export function formatDateTime(value: string | null | undefined): string {
  if (!value) return 'Unknown';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return 'Unknown';
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date);
}

interface VersionParts {
  core: number[];
  pre: string | null;
}

function splitVersion(version: string): VersionParts {
  const dash = version.indexOf('-');
  const core = (dash === -1 ? version : version.slice(0, dash))
    .split('.')
    .map((part) => {
      const parsed = Number.parseInt(part, 10);
      return Number.isNaN(parsed) ? 0 : parsed;
    });
  return { core, pre: dash === -1 ? null : version.slice(dash + 1) };
}

/**
 * Sort comparator for dotted version strings, newest first.
 *
 * Numeric segments are compared pairwise; a release sorts before a prerelease
 * of the same core (e.g. `1.0.0` is newer than `1.0.0-beta.1`), and
 * prerelease tags compare numeric-aware (`beta.10` > `beta.2`). Pinned loader
 * versions (`0.16.9`, `50.1.0`) are plain numeric triples, but the prerelease
 * path keeps the sort stable if one ever appears.
 */
/**
 * Lifetime play time as a short label: "45 min", "3.5 h", "120 h". Returns
 * null for nothing recorded, so callers can leave the label out entirely.
 */
export function formatPlayTime(ms: number | null | undefined): string | null {
  if (!ms || ms < 60_000) return null;
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 60) return `${minutes} min`;
  const hours = ms / 3_600_000;
  return hours < 10 ? `${(Math.floor(hours * 10) / 10).toString()} h` : `${Math.floor(hours)} h`;
}

export function compareVersionsDescending(a: string, b: string): number {
  const va = splitVersion(a);
  const vb = splitVersion(b);
  const length = Math.max(va.core.length, vb.core.length);
  for (let i = 0; i < length; i += 1) {
    const diff = (vb.core[i] ?? 0) - (va.core[i] ?? 0);
    if (diff !== 0) return diff;
  }
  if (va.pre !== null && vb.pre === null) return 1;
  if (va.pre === null && vb.pre !== null) return -1;
  if (va.pre !== null && vb.pre !== null) {
    return vb.pre.localeCompare(va.pre, undefined, { numeric: true });
  }
  return 0;
}

/** Copy of `versions` sorted newest-first by `loader_version` (stable order kept). */
export function sortLoaderVersionsLatestFirst<T extends { loader_version: string }>(versions: T[]): T[] {
  return [...versions].sort((a, b) => compareVersionsDescending(a.loader_version, b.loader_version));
}
