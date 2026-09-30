import type { BrowseItemCached } from '../lib/tauri';

/**
 * A snapshot of the Browse list, kept alive across navigation into a mod
 * detail page and back.
 *
 * Browse used to rely purely on staying mounted while ModDetail was showing
 * (`shouldRenderBrowse` in App.tsx). That works only until the next App
 * re-render: the condition reads `previousDestinationRef`, which an effect
 * rewrites to `mod-detail` immediately after navigating, so any subsequent App
 * state change unmounts Browse and throws away the loaded pages and scroll
 * position. Persisting the list here makes returning cheap and correct whether
 * or not the component survived, and covers the Bazaar too — its shelf order is
 * derived from the same item list.
 *
 * Deliberately module-level rather than React state: it must outlive the
 * component, and it is per-session scratch, not something to persist to disk.
 */
export interface BrowseSnapshot {
  /** Identity of the filters that produced this list. */
  queryKey: string;
  items: BrowseItemCached[];
  hasMore: boolean;
  /** 0-indexed page most recently loaded, so load-more resumes correctly. */
  currentPage: number;
  /** Scroll offset of the app's scroll container when the user navigated away. */
  scrollTop: number;
  /** Whether the user was in the Bazaar, so returning restores the same view. */
  bazaarMode: boolean;
  /** Bazaar shelf order already shown, so returning does not reshuffle it. */
  bazaarSettledOrder: string[];
}

let snapshot: BrowseSnapshot | null = null;
/**
 * Only a snapshot explicitly parked on the way to a detail page may be
 * restored. Without this the plain "same query" check also matches a first
 * load or a same-key effect re-run, and Browse would skip its initial fetch
 * and render an empty list.
 */
let parked = false;

/** Store (or replace) the current Browse list state. */
export function saveBrowseSnapshot(next: BrowseSnapshot): void {
  snapshot = next;
}

/** Read the stored snapshot without consuming it. */
export function peekBrowseSnapshot(): BrowseSnapshot | null {
  return snapshot;
}

/**
 * Mark the snapshot as the state to come back to, recording where the user was
 * scrolled. Called when navigating into a mod detail page.
 */
export function parkBrowseSnapshot(scrollTop: number): void {
  if (!snapshot) return;
  snapshot = { ...snapshot, scrollTop };
  parked = true;
}

/**
 * Consume a parked snapshot, but only when it belongs to the same query. A
 * changed sort/filter/search falls through to a fresh fetch rather than showing
 * a list that no longer matches the controls.
 */
export function takeBrowseSnapshot(queryKey: string): BrowseSnapshot | null {
  if (!parked || !snapshot || snapshot.queryKey !== queryKey) return null;
  parked = false;
  return snapshot;
}

/** Drop the snapshot — used when the query changes or results are invalidated. */
export function clearBrowseSnapshot(): void {
  snapshot = null;
  parked = false;
}

/** The Minecraft version and loader Browse was filtered to. */
export interface BrowseFilterContext {
  mcVersion: string | null;
  loader: string | null;
}

/**
 * The filters of the Browse list the user just opened a detail page from, or
 * null when this page was not reached from Browse (only a parked snapshot
 * counts, so a stale list from earlier in the session is never mistaken for
 * the current context).
 */
export function peekParkedBrowseFilter(): BrowseFilterContext | null {
  if (!parked || !snapshot) return null;
  try {
    const key = JSON.parse(snapshot.queryKey) as { mcVersion?: unknown; loader?: unknown };
    const mcVersion = typeof key.mcVersion === 'string' && key.mcVersion ? key.mcVersion : null;
    const loader = typeof key.loader === 'string' && key.loader ? key.loader : null;
    return mcVersion || loader ? { mcVersion, loader } : null;
  } catch {
    return null;
  }
}

/**
 * The release a pack should default to: the newest (first) one matching the
 * Browse filter, else the newest overall. `mismatch` is true when the chosen
 * release targets a different Minecraft version than the filter.
 */
export function pickDefaultPackRelease<T extends { minecraft_version: string; loader: string }>(
  releases: readonly T[],
  filter: BrowseFilterContext | null,
): { index: number; mismatch: boolean } {
  if (releases.length === 0) return { index: -1, mismatch: false };
  const mc = filter?.mcVersion ?? null;
  const loader = filter?.loader?.toLowerCase() ?? null;
  if (!mc && !loader) return { index: 0, mismatch: false };
  const matches = (release: T, withLoader: boolean) =>
    (!mc || release.minecraft_version === mc)
    && (!withLoader || !loader || release.loader.toLowerCase() === loader);
  let index = releases.findIndex((release) => matches(release, true));
  if (index < 0) index = releases.findIndex((release) => matches(release, false));
  if (index >= 0) return { index, mismatch: false };
  return { index: 0, mismatch: !!mc };
}
