/**
 * Visual identity for an instance — the colours and letters of its tile.
 *
 * Instances without a custom image used to render no icon at all, so the grid
 * was a wall of text. Every instance gets a tile now, and for the ones with no
 * image it has to be generated. Two constraints shape how:
 *
 *  - It must be *stable*. The same instance produces the same tile on every
 *    render and every restart, with nothing persisted. So it is derived, not
 *    picked.
 *  - It must be *informative*. Hue family comes from the modloader, so the
 *    grid groups by loader at a glance; a hash of the instance id then shifts
 *    hue and gradient angle inside that family, so two Fabric instances are
 *    still told apart.
 *
 * The hues are literal HSL rather than theme tokens on purpose. The job here
 * is telling instances apart, which a single themed accent cannot do. Every
 * *surface* around the tile stays token-driven as usual.
 */

/** Base hue per loader, spread far enough apart to survive the ±14° jitter. */
const LOADER_HUE: Record<string, number> = {
  neoforge: 12, // red-orange
  fabric: 45, // the loader's tan/gold
  vanilla: 104, // grass
  forge: 212, // anvil steel
  quilt: 288, // violet
};

/** Unknown or future loaders get their own family rather than borrowing one. */
const FALLBACK_HUE = 170; // teal — the widest gap left between the five above

export function normalizeLoader(loader: string | null | undefined): string {
  return (loader ?? '').trim().toLowerCase();
}

/** Loader-family hue on its own, for chips and dots that sit beside a tile. */
export function loaderHue(loader: string | null | undefined): number {
  return LOADER_HUE[normalizeLoader(loader)] ?? FALLBACK_HUE;
}

/** FNV-1a: small, dependency-free, and identical on every platform. */
function hash32(value: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < value.length; i += 1) {
    h ^= value.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

export interface InstanceTint {
  /** Gradient start hue, inside the loader's family. */
  hueA: number;
  /** Gradient end hue. */
  hueB: number;
  /** Gradient angle in degrees. */
  angle: number;
}

export function instanceTint(seed: string, loader?: string | null): InstanceTint {
  const base = LOADER_HUE[normalizeLoader(loader)] ?? FALLBACK_HUE;
  const h = hash32(seed || 'instance');
  const jitter = (h % 23) - 11; // −11…+11, narrow enough that families never meet
  const spread = 24 + ((h >>> 5) % 22); // 24…45 between the two stops
  const angle = 110 + ((h >>> 11) % 70); // 110…179deg
  const hueA = (base + jitter + 360) % 360;
  return { hueA, hueB: (hueA + spread) % 360, angle };
}

/**
 * Up to two letters for the tile. Words win over characters ("Sky Factory" →
 * "SF"), and punctuation is dropped first so "~ modpack!" still reads as "MO".
 */
export function instanceInitials(name: string): string {
  const words = (name ?? '').split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  if (words.length === 0) return '?';
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return (words[0][0] + words[1][0]).toUpperCase();
}
