/**
 * The Bazaar model — the five-axis taste model, the shelf scoring, and the
 * "Surprise me" machine, kept pure so the recommender stays legible and
 * testable.
 *
 * Rules this model must keep:
 *  - The taste model stays legible: the bars exist so the user can see why
 *    the order changed. Never replace it with an opaque score.
 *  - Owned items sort to the back (`scoreOf` returns −99 for anything in the
 *    bag/staged set) so the shelf keeps offering new things.
 *  - "Surprise me" is weighted by taste and never returns something already
 *    owned; an unweighted random gets annoying by the third crank.
 *  - Taste is linear and additive.
 */

/**
 * Taste axes are REAL Modrinth category slugs (adventure, decoration, magic,
 * technology, optimization, utility, worldgen, mobs) — not invented words.
 * The prototype's five fictional vibes were replaced because the labels were
 * meaningless; these are the categories Agora actually ships with, so the
 * bars and the reorder explain themselves.
 */
export const VIBES = ['adventure', 'decoration', 'magic', 'technology', 'optimization', 'utility', 'worldgen', 'mobs'] as const;

export type Vibe = (typeof VIBES)[number];

export const VIBE_LABEL: Record<Vibe, string> = {
  adventure: 'Adventure',
  decoration: 'Decoration',
  magic: 'Magic',
  technology: 'Technology',
  optimization: 'Optimization',
  utility: 'Utility',
  worldgen: 'Worldgen',
  mobs: 'Mobs',
};

export interface BazaarItem {
  id: string;
  name: string;
  iconUrl: string | null;
  description: string | null;
  contentType: string;
  author: string | null;
  /** Real Modrinth category slugs (the source of the taste axes + tags). */
  categories: string[];
  supportedVersions: string[];
  /** True when this item comes from the curated registry (gets the shiny
   * curated tint + CURATED tag; everything else stays plain). */
  curated?: boolean;
  /** Curatorial signals for sorting/interest. */
  downloads?: number;
  follows?: number;
}

export type VoteDirection = -1 | 0 | 1;

export interface BazaarState {
  taste: Record<Vibe, number>;
  votes: Record<string, VoteDirection>;
  /** Owned (installed in the target instance) — sort to back, never gacha. */
  owned: Record<string, boolean>;
  /** Staged this session ("put it in my bag") — same treatment as owned. */
  staged: Record<string, boolean>;
}

export function initialBazaarState(): BazaarState {
  return {
    taste: Object.fromEntries(VIBES.map((v) => [v, 0])) as Record<Vibe, number>,
    votes: {},
    owned: {},
    staged: {},
  };
}

/** Whether an item is effectively "owned" (installed or staged). */
export function isOwned(state: BazaarState, id: string): boolean {
  return !!state.owned[id] || !!state.staged[id];
}

/**
 * The taste axes ARE the real Modrinth categories: an item's axes are exactly
 * the categories it is tagged with (intersected with the axis set), so the
 * recommender can explain itself with the catalog's own vocabulary.
 */
export function vibesFor(item: Pick<BazaarItem, 'categories'>): Vibe[] {
  const found = new Set<Vibe>();
  for (const category of item.categories ?? []) {
    if ((VIBES as readonly string[]).includes(category)) found.add(category as Vibe);
  }
  // Every item has at least one axis so the shelf is never unjudgeable.
  if (found.size === 0) found.add('utility');
  return VIBES.filter((v) => found.has(v));
}

/** Capitalise a modrinth category slug for display ("game-mechanics" → "Game mechanics"). */
export function formatCategory(value: string): string {
  return value.replace(/[-_]/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
}

/** The real Modrinth category tags shown on a tile (capped, deduped). */
export function categoryTags(item: Pick<BazaarItem, 'categories'>): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const c of item.categories ?? []) {
    const label = formatCategory(c);
    if (seen.has(label)) continue;
    seen.add(label);
    out.push(label);
    if (out.length >= 4) break;
  }
  return out;
}

/**
 * Prototype `scoreOf`, verbatim in intent:
 *   score = sum of the item's vibes' current taste values
 *         − 99 if already owned/staged (sorts to the back)
 */
export function scoreOf(state: BazaarState, item: BazaarItem): number {
  let score = 0;
  for (const v of vibesFor(item)) score += state.taste[v] || 0;
  // No curated nudge here any more: the backend now ranks curated content into
  // its own band, and `sortedShelf` tiebreaks on backend order, so adding a
  // second bonus would double-count the same preference.
  return score + (isOwned(state, item.id) ? -99 : 0);
}

/**
 * Prototype `vote`: ❤️ ("more like this") = +1 to each of the item's vibes,
 * 💔 ("not for me") = −1; tapping the
 * same button again cancels (returns to 0). Returns the new state.
 */
export function vote(state: BazaarState, item: BazaarItem, direction: VoteDirection): BazaarState {
  const prev = state.votes[item.id] ?? 0;
  const next = prev === direction ? 0 : direction;
  const taste = { ...state.taste };
  for (const v of vibesFor(item)) taste[v] = (taste[v] || 0) - prev + next;
  return { ...state, taste, votes: { ...state.votes, [item.id]: next } };
}

/** Mark an item staged ("put it in my bag" → the reviewed install flow). */
export function stageItem(state: BazaarState, item: BazaarItem): BazaarState {
  if (isOwned(state, item.id)) return state;
  return { ...state, staged: { ...state.staged, [item.id]: true } };
}

export function unstageItem(state: BazaarState, id: string): BazaarState {
  if (!state.staged[id]) return state;
  const staged = { ...state.staged };
  delete staged[id];
  return { ...state, staged };
}

/** The vibe whose current taste is strongest (for the "Looks like you're
 * after …" note). Returns null when everything is flat. */
export function topVibe(state: BazaarState): Vibe | null {
  let best: Vibe | null = null;
  let bestValue = 0;
  for (const v of VIBES) {
    if (state.taste[v] > bestValue) { best = v; bestValue = state.taste[v]; }
  }
  return best;
}

/** Normalized bar widths (prototype `renderBars`): width = max(0, val) / maxAbs. */
export function vibeBarWidths(state: BazaarState): Record<Vibe, number> {
  const maxAbs = Math.max(1, ...VIBES.map((v) => Math.abs(state.taste[v])));
  return Object.fromEntries(
    VIBES.map((v) => [v, Math.max(0, state.taste[v]) / maxAbs * 100]),
  ) as Record<Vibe, number>;
}

/**
 * The "Surprise me" machine (prototype `crank`): weighted by taste so it
 * still surprises but rarely annoys, and never returns something already
 * owned. `max(0.35, 1 + scoreOf(it))` keeps a floor so low-scored items are
 * still possible.
 */
export function crank(state: BazaarState, pool: BazaarItem[]): BazaarItem | null {
  const candidates = pool.filter((it) => !isOwned(state, it.id));
  if (candidates.length === 0) return null;
  const weights = candidates.map((it) => Math.max(0.35, 1 + scoreOf(state, it)));
  const total = weights.reduce((a, b) => a + b, 0);
  let r = Math.random() * total;
  let pick = candidates[0];
  for (let i = 0; i < candidates.length; i++) {
    r -= weights[i];
    if (r <= 0) { pick = candidates[i]; break; }
  }
  return pick;
}

/**
 * Sort the shelf: taste score desc, then **the order the backend gave us**.
 *
 * The incoming array is already sorted by the selected Browse sort (net score by
 * default), so falling back to the original index means the Bazaar agrees with
 * the Browse page whenever taste is neutral — and taste starts neutral for
 * everyone. Tie-breaking on `name` instead made a brand-new shelf look strictly
 * alphabetical, which reads as "no ranking at all" and buries the good stuff
 * under whatever starts with "A". This also keeps whichever sort the user picked
 * meaningful, rather than hardcoding one.
 */
export function sortedShelf(
  state: BazaarState,
  items: BazaarItem[],
  settledOrder: string[] = [],
): BazaarItem[] {
  const rank = new Map(items.map((it, i) => [it.id, i]));
  const byScore = (a: BazaarItem, b: BazaarItem) => {
    const d = scoreOf(state, b) - scoreOf(state, a);
    return d !== 0 ? d : (rank.get(a.id) ?? 0) - (rank.get(b.id) ?? 0);
  };

  // Items the shelf has already shown keep the order they were shown in.
  // Re-sorting them as taste accumulates made cards jump positions while the
  // user was scrolling, so an item could be scrolled past before it moved into
  // view — the reason a mod could be missed entirely.
  const settled = new Set(settledOrder);
  const kept = settledOrder
    .map((id) => items.find((it) => it.id === id))
    .filter((it): it is BazaarItem => Boolean(it));
  const incoming = items.filter((it) => !settled.has(it.id)).sort(byScore);
  return [...kept, ...incoming];
}

/** Stall categories (the prototype's CATS), driven by the real content types. */
export interface Stall {
  id: string;
  label: string;
  emoji: string;
}

export const STALLS: Stall[] = [
  { id: 'all', label: 'Everything', emoji: '🧺' },
  { id: 'mod', label: 'Mods', emoji: '🧩' },
  { id: 'pack', label: 'Modpacks', emoji: '🎁' },
  { id: 'shader', label: 'Shaders', emoji: '✨' },
  { id: 'resourcepack', label: 'Resource packs', emoji: '🎨' },
  { id: 'world', label: 'Worlds to explore', emoji: '🗺️' },
  { id: 'datapack', label: 'Data packs', emoji: '⚙️' },
];

/* Blocky stall icons (the prototype's `ICON`), painted on a canvas. */
export type StallIcon = (c: CanvasRenderingContext2D, w: number, h: number) => void;

function iconPx(c: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, col: string): void {
  c.fillStyle = col;
  c.fillRect(x, y, w, h);
}

export const STALL_ICONS: Record<string, StallIcon> = {
  all: (c, w, h) => {
    const b = Math.min(w, h) / 5;
    iconPx(c, b, b, b * 1.4, b * 1.4, '#6FD3E8');
    iconPx(c, b * 2.6, b, b * 1.4, b * 1.4, '#8BE24F');
    iconPx(c, b, b * 2.6, b * 1.4, b * 1.4, '#FFB25E');
    iconPx(c, b * 2.6, b * 2.6, b * 1.4, b * 1.4, '#B48CF2');
  },
  mod: (c, w, h) => {
    const b = Math.min(w, h) / 5;
    iconPx(c, b * 0.8, b * 1.2, b * 1.8, b * 1.8, '#6FD3E8');
    iconPx(c, b * 2.4, b * 1.2, b * 1.8, b * 1.8, '#B48CF2');
    iconPx(c, b * 2.4, b * 1.9, b * 0.6, b * 0.6, '#0A1820');
  },
  pack: (c, w, h) => {
    const b = Math.min(w, h) / 5;
    iconPx(c, b * 0.9, b * 1.5, b * 3.2, b * 2.2, '#B08A4A');
    iconPx(c, b * 1.8, b * 0.9, b * 1.4, b * 0.7, '#8A6A34');
  },
  shader: (c, w, h) => {
    const b = Math.min(w, h) / 5;
    iconPx(c, b * 0.8, b * 2.6, b * 3.4, b * 1.2, '#3A7CB0');
    iconPx(c, b * 1.6, b * 0.9, b * 1.8, b * 1.8, '#FFD34E');
  },
  resourcepack: (c, w, h) => {
    const b = Math.min(w, h) / 5;
    iconPx(c, b * 0.8, b * 2.6, b * 3.4, b * 1.2, '#3A7CB0');
    iconPx(c, b * 1.6, b * 0.9, b * 1.8, b * 1.8, '#FFD34E');
  },
  world: (c, w, h) => {
    const b = Math.min(w, h) / 5;
    iconPx(c, b * 0.9, b * 2.2, b * 3.2, b * 1.6, '#4C8F4F');
    iconPx(c, b * 1.4, b * 1.4, b * 0.9, b * 0.9, '#8BE24F');
    iconPx(c, b * 2.6, b * 1.1, b * 0.9, b * 1.2, '#6B4A2E');
  },
  datapack: (c, w, h) => {
    const b = Math.min(w, h) / 5;
    iconPx(c, b * 0.9, b * 1.4, b * 3.2, b * 2.2, '#B08A4A');
    iconPx(c, b * 1.7, b * 1.9, b * 1.4, b * 1.2, '#6FD3E8');
  },
};

/** The gacha machine's idle art (prototype `gachaArt`). */
export function gachaMachineArt(c: CanvasRenderingContext2D, w: number, h: number): void {
  const b = Math.min(w, h) / 9, cx = w / 2, base = h - b * 0.6;
  iconPx(c, cx - b * 2.6, base - b * 7, b * 5.2, b * 4, '#2A4C5E'); // glass dome
  ['#FF7A6B', '#FFD34E', '#8BE24F', '#6FD3E8', '#B48CF2'].forEach((col, i) => {
    iconPx(c, cx - b * 2 + (i % 3) * b * 1.5, base - b * 6.2 + Math.floor(i / 3) * b * 1.4, b * 1.2, b * 1.2, col);
  });
  iconPx(c, cx - b * 3, base - b * 3, b * 6, b * 3, '#B0453A'); // body
  iconPx(c, cx - b * 1, base - b * 2.2, b * 2, b * 1.6, '#1A2630'); // chute
  iconPx(c, cx + b * 2.2, base - b * 2.2, b * 1.2, b * 1.2, '#FFD34E'); // crank
}

export function matchesStall(item: BazaarItem, stallId: string): boolean {
  if (stallId === 'all') return true;
  return item.contentType === stallId;
}

/**
 * Deterministic monogram creature art: the same name always yields the same
 * creature, so the shelf feels like a place with fixed inhabitants rather
 * than random noise. Used only when the real `icon_url` is missing —
 * creature art is a fallback, not the design.
 */
export function hashOf(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return Math.abs(h);
}

/** Paint a critter on a small canvas (prototype `critter`, blocky rects). */
export function paintCritter(ctx: CanvasRenderingContext2D, name: string, w: number, h: number, hue?: number): void {
  const s = hashOf(name);
  const b = Math.min(w, h) / 11;
  const cx = w / 2, cy = h / 2 + b * 0.6;
  const hu = hue !== undefined ? hue : (s % 360);
  const body = `hsl(${hu} 58% 52%)`, dark = `hsl(${hu} 60% 34%)`, light = `hsl(${hu} 70% 72%)`;
  const px = (x: number, y: number, pw: number, ph: number, color: string) => {
    ctx.fillStyle = color;
    ctx.fillRect(Math.round(x), Math.round(y), Math.max(1, Math.round(pw)), Math.max(1, Math.round(ph)));
  };
  // body
  px(cx - b * 3, cy - b * 1.6, b * 6, b * 3.4, body);
  // head
  px(cx - b * 4.6, cy - b * 3.2, b * 3.2, b * 3, body);
  px(cx - b * 4.2, cy - b * 2.4, b * 0.7, b * 0.7, '#0A1820'); // eye
  // ears / horns vary by hash
  if (s % 3 === 0) { px(cx - b * 4.2, cy - b * 4.4, b * 0.8, b * 1.3, dark); px(cx - b * 2.6, cy - b * 4.4, b * 0.8, b * 1.3, dark); }
  else if (s % 3 === 1) { px(cx - b * 4.4, cy - b * 4.1, b * 2.4, b * 0.9, dark); }
  // legs
  px(cx - b * 2.4, cy + b * 1.8, b * 1, b * 1.7, dark);
  px(cx + b * 1.4, cy + b * 1.8, b * 1, b * 1.7, dark);
  // tail
  if (s % 2 === 0) px(cx + b * 3, cy - b * 1.2, b * 1.6, b * 0.9, light);
  else px(cx + b * 2.9, cy - b * 2.4, b * 0.9, b * 2.2, light);
  // little sparkle so the tile has life
  px(cx + b * 1.2, cy - b * 3.4, b * 0.6, b * 0.6, light);
}

/** The fit line: the same version-compatibility question the pre-flight asks,
 * answered before you commit. `null` when there is no instance context to
 * check against (the line is then neutral, not a claim). */
export function fitFor(item: BazaarItem, instanceVersion: string | null): boolean | null {
  if (!instanceVersion) return null;
  if (!item.supportedVersions || item.supportedVersions.length === 0) return null;
  return item.supportedVersions.includes(instanceVersion);
}

export const STORAGE_KEY = 'agora-bazaar-v1';

export function loadBazaarState(): BazaarState {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return initialBazaarState();
    const parsed = JSON.parse(raw) as Partial<BazaarState>;
    // Taste defaults now cover the real Modrinth category axes; the old
    // fictional vibe keys are simply ignored (harmless drop).
    const defaults = initialBazaarState().taste;
    return {
      taste: { ...defaults, ...(parsed.taste ?? {}) },
      votes: parsed.votes ?? {},
      owned: parsed.owned ?? {},
      staged: parsed.staged ?? {},
    };
  } catch {
    return initialBazaarState();
  }
}

export function saveBazaarState(state: BazaarState): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // best effort
  }
}
