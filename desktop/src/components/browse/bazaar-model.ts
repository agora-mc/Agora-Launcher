/**
 * The Bazaar model — the five-axis taste model, the shelf scoring, and the
 * "Surprise me" machine, ported from `v5-browse.html` (§11) and kept pure so
 * the recommender stays legible and testable.
 *
 * Rules the port must keep (V5-PORT-PLAN §11):
 *  - The taste model stays legible: the bars exist so the user can see why
 *    the order changed. Never replace it with an opaque score.
 *  - Owned items sort to the back (`scoreOf` returns −99 for anything in the
 *    bag/staged set) so the shelf keeps offering new things.
 *  - "Surprise me" is weighted by taste and never returns something already
 *    owned; an unweighted random gets annoying by the third crank.
 *  - Taste is linear and additive, exactly as the prototype.
 */

export type Vibe = 'cosy' | 'wild' | 'silly' | 'pretty' | 'tricky';

export const VIBES: Vibe[] = ['cosy', 'wild', 'silly', 'pretty', 'tricky'];

export const VIBE_LABEL: Record<Vibe, string> = {
  cosy: 'Cosy',
  wild: 'Wild',
  silly: 'Silly',
  pretty: 'Pretty',
  tricky: 'Tricky',
};

export interface BazaarItem {
  id: string;
  name: string;
  iconUrl: string | null;
  description: string | null;
  contentType: string;
  author: string | null;
  categories: string[];
  supportedVersions: string[];
  /** Marked popular (the prototype's `hot` ribbon). */
  popular?: boolean;
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
    taste: { cosy: 0, wild: 0, silly: 0, pretty: 0, tricky: 0 },
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
 * Map a curated item's categories + content type onto the five taste axes.
 * The mapping is deliberately transparent (a small table) so the recommender
 * can explain itself — the plan's "sorting you can see".
 */
const CATEGORY_VIBES: Array<[RegExp, Vibe[]]> = [
  [/building|decoration|furniture|furnish|cosmetic|aesthetic|visual|quality of life|qol/i, ['cosy', 'pretty']],
  [/adventure|worldgen|biome|creature|animal|exploration|dimension|terrain|nature/i, ['wild']],
  [/fun|funny|meme|joke|silly|challenge/i, ['silly']],
  [/shader|graphic|texture|resource|hd|realistic|beauty/i, ['pretty']],
  [/technology|magic|mechanic|automation|storage|redstone|combat|difficulty|tech/i, ['tricky']],
];

const CONTENT_TYPE_VIBES: Record<string, Vibe[]> = {
  shader: ['pretty'],
  resourcepack: ['pretty', 'cosy'],
  world: ['wild'],
  datapack: ['tricky'],
  server: ['wild', 'tricky'],
};

export function vibesFor(item: Pick<BazaarItem, 'contentType' | 'categories'>): Vibe[] {
  const found = new Set<Vibe>();
  (CONTENT_TYPE_VIBES[item.contentType] ?? []).forEach((v) => found.add(v));
  for (const category of item.categories ?? []) {
    for (const [re, vibes] of CATEGORY_VIBES) {
      if (re.test(category)) vibes.forEach((v) => found.add(v));
    }
  }
  // Every item has at least one axis so the shelf is never unjudgeable.
  if (found.size === 0) found.add('cosy');
  return VIBES.filter((v) => found.has(v));
}

/**
 * Prototype `scoreOf`, verbatim in intent:
 *   score = sum of the item's vibes' current taste values
 *         + 0.6 if popular
 *         − 99 if already owned/staged (sorts to the back)
 */
export function scoreOf(state: BazaarState, item: BazaarItem): number {
  let score = 0;
  for (const v of vibesFor(item)) score += state.taste[v] || 0;
  return score + (item.popular ? 0.6 : 0) + (isOwned(state, item.id) ? -99 : 0);
}

/**
 * Prototype `vote`: 👍 = +1 to each of the item's vibes, 👎 = −1; tapping the
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

/** Sort the shelf: score desc, then name for stability. */
export function sortedShelf(state: BazaarState, items: BazaarItem[]): BazaarItem[] {
  return items.slice().sort((a, b) => {
    const d = scoreOf(state, b) - scoreOf(state, a);
    return d !== 0 ? d : a.name.localeCompare(b.name);
  });
}

/** Stall categories (the prototype's CATS), driven by the real content types. */
export interface Stall {
  id: string;
  label: string;
  emoji: string;
}

export const STALLS: Stall[] = [
  { id: 'all', label: 'Everything', emoji: '🧺' },
  { id: 'mod', label: 'Add-ons', emoji: '🧩' },
  { id: 'pack', label: 'Ready-made worlds', emoji: '🎁' },
  { id: 'shader', label: 'Make it pretty', emoji: '✨' },
  { id: 'resourcepack', label: 'Looks & textures', emoji: '🎨' },
  { id: 'world', label: 'Maps to explore', emoji: '🗺️' },
  { id: 'datapack', label: 'Tricky extras', emoji: '⚙️' },
];

export function matchesStall(item: BazaarItem, stallId: string): boolean {
  if (stallId === 'all') return true;
  return item.contentType === stallId;
}

/**
 * Deterministic monogram creature art (prototype `hashOf`/`critter`): the same
 * name always yields the same creature, so the shelf feels like a place with
 * fixed inhabitants rather than random noise. Used only when the real
 * `icon_url` is missing (plan §11: creature art is a fallback, not the design).
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
    return {
      taste: { cosy: 0, wild: 0, silly: 0, pretty: 0, tricky: 0, ...(parsed.taste ?? {}) },
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
