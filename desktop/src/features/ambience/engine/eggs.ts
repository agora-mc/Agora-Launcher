/**
 * The 54-egg discovery registry + achievement logic, ported from v4-world.html.
 *
 * Eggs live in ambience (V5-PORT-PLAN §2): the world stays discoverable on
 * every page. The engine keeps the registry + find/achievement logic; the
 * React side renders toasts and the Field Journal from `journalData()`.
 */

import type { EngineState } from './state';
import type { WorldState } from './types';

export interface Egg {
  id: string;
  name: string;
  tier: 1 | 2 | 3;
  hint: string;
}

export const EGGS: Egg[] = [
  // tier 1 — single click (26)
  { id: 'fox-nap', name: 'Sleepy Fox', tier: 1, hint: 'Some animals get tired when bothered.' },
  { id: 'deer-stare', name: 'Deer in Headlights', tier: 1, hint: 'Deer freeze when they notice you.' },
  { id: 'bear-roar', name: 'Bear Necessities', tier: 1, hint: "Bears aren't shy about being bothered." },
  { id: 'bird-song', name: 'Dawn Chorus', tier: 1, hint: 'Something small sings in the morning sky.' },
  { id: 'goose-honk', name: 'Honk', tier: 1, hint: "Geese travel in a V. They're loud about it." },
  { id: 'gull-scream', name: 'Sky Rat', tier: 1, hint: 'It hovers over the coastline, screeching.' },
  { id: 'hog-ball', name: 'Prickle Ball', tier: 1, hint: 'A slow waddler that curls up when startled.' },
  { id: 'wolf-howl', name: 'Lone Howl', tier: 1, hint: 'Wolves only prowl after dark.' },
  { id: 'squirrel-nut', name: 'Nut Job', tier: 1, hint: 'It darts, stops, and climbs trees.' },
  { id: 'rabbit-thump', name: 'Thumper', tier: 1, hint: 'Hops in quick bursts.' },
  { id: 'owl-spin', name: 'Who?', tier: 1, hint: 'Perched and still, only after dark.' },
  { id: 'bat-loop', name: 'Echolocation', tier: 1, hint: 'Erratic flight, only after dark.' },
  { id: 'frog-jump', name: 'Ribbit', tier: 1, hint: 'Sits on a lily pad.' },
  { id: 'turtle-hide', name: 'Shell Shock', tier: 1, hint: 'Extremely slow, extremely defensive.' },
  { id: 'butterfly-land', name: 'Landing Pad', tier: 1, hint: 'Flutters near the flowers.' },
  { id: 'firefly-sync', name: 'Synchrony', tier: 1, hint: 'Click a few — they glow after dark.' },
  { id: 'crow-scatter', name: 'A Murder', tier: 1, hint: 'Perches on the scarecrow or fence.' },
  { id: 'pecker-hole', name: 'Knock Knock', tier: 1, hint: 'Clings to a tree, pecking rhythmically.' },
  { id: 'raccoon-guilt', name: 'Trash Panda', tier: 1, hint: 'Rummages around, only after dark.' },
  { id: 'moose-bellow', name: 'Timber!', tier: 1, hint: 'Very slow, very large. A rare sighting.' },
  { id: 'boar-truffle', name: 'Truffle Shuffle', tier: 1, hint: 'Snuffles along the ground.' },
  { id: 'mouse-hide', name: 'Squeak', tier: 1, hint: 'Very fast, hugs the ground.' },
  { id: 'flower-bloom', name: 'Bloom', tier: 1, hint: 'Click a patch of flowers.' },
  { id: 'pond-fish', name: "Something's Biting", tier: 1, hint: 'Click the pond itself.' },
  { id: 'rock-beetle', name: 'Under a Rock', tier: 1, hint: "Click a rock and see what's under it." },
  { id: 'log-shroom', name: 'Fungus Among Us', tier: 1, hint: 'Click the fallen log.' },
  // tier 2 — two steps (14)
  { id: 'acorn-squirrel', name: 'Special Delivery', tier: 2, hint: 'Shake a tree for an acorn, then find a squirrel.' },
  { id: 'berry-hog', name: 'Berry Nice', tier: 2, hint: 'Pick berries from a bush, then find a hedgehog.' },
  { id: 'flower-deer', name: 'Flower Power', tier: 2, hint: 'Pick a flower, then find a deer.' },
  { id: 'fish-otter', name: 'Sharing is Caring', tier: 2, hint: 'Catch a fish, then find an otter in the pond.' },
  { id: 'pinecone-squirrel', name: 'Pinecone Post', tier: 2, hint: 'Shake a pine tree, then find a squirrel.' },
  { id: 'feather-scarecrow', name: 'Dapper', tier: 2, hint: 'A seagull drops a feather. Scarecrows love hats.' },
  { id: 'truffle-pond', name: 'Plop', tier: 2, hint: 'A boar digs up something. Ponds like surprises.' },
  { id: 'honey-bear', name: 'Sweet Tooth', tier: 2, hint: 'Click the beehive three times, then find the bear.' },
  { id: 'firefly-cave', name: 'Night Light', tier: 2, hint: 'Catch a firefly and carry it somewhere dark.' },
  { id: 'snail-lily', name: 'Snail Mail', tier: 2, hint: "A snail, carried to the water's edge." },
  { id: 'duck-line', name: 'Duck Duck Goose', tier: 2, hint: 'Click a duck several times in a row.' },
  { id: 'acorn-pond', name: 'Wishing Well', tier: 2, hint: 'An acorn, thrown into the pond.' },
  { id: 'bee-hive', name: 'Bee Line', tier: 2, hint: 'Follow a few different bees back to where they came from.' },
  { id: 'boulder-hole', name: 'Rolling Stone', tier: 2, hint: "A boulder won't move on the first try." },
  // tier 3 — multi-step (14)
  { id: 'bear-fish', name: "Fisherman's Friend", tier: 3, hint: 'Bears are hungry. Ponds have food.' },
  { id: 'snowman', name: 'Frosty', tier: 3, hint: 'In snow, build from the biggest pile down.' },
  { id: 'constellation', name: 'Stargazer', tier: 3, hint: 'At night, the brightest star goes first.' },
  { id: 'bear-feast', name: 'The Great Feast', tier: 3, hint: 'Three courses for one bear.' },
  { id: 'acorn-hunt', name: 'Green Thumb', tier: 3, hint: 'Plant acorns in three spots, then wait a day.' },
  { id: 'fairy-ring', name: 'Fairy Ring', tier: 3, hint: 'All seven mushrooms, quickly.' },
  { id: 'rainbow', name: "Rainbow's End", tier: 3, hint: 'Rain, then sun — during the day.' },
  { id: 'wolf-pack', name: 'Pack Leader', tier: 3, hint: 'Answer every call before they leave.' },
  { id: 'campfire-tales', name: 'Campfire Stories', tier: 3, hint: 'Warmth draws a crowd, after dark.' },
  { id: 'moonlit-rave', name: 'Moonlit Rave', tier: 3, hint: 'Gather the lights, after dark.' },
  { id: 'the-long-con', name: 'The Long Con', tier: 3, hint: 'Make a friend. Friends fetch things.' },
  { id: 'migration', name: 'Migration', tier: 3, hint: 'Lead the leader — click it more than once.' },
  { id: 'water-flowers', name: 'Watering Can', tier: 3, hint: 'Flowers are thirsty. Ponds have water.' },
  { id: 'full-day', name: 'Time Traveler', tier: 3, hint: 'Just watch the time slider for a while.' },
];

export const EGG_BY_ID: Record<string, Egg> = {};
EGGS.forEach((e) => { EGG_BY_ID[e.id] = e; });

export const NIGHT_EGG_IDS = ['owl-spin', 'wolf-howl', 'bat-loop', 'firefly-sync', 'raccoon-guilt', 'constellation', 'campfire-tales', 'moonlit-rave'];

export const TIER_NAME: Record<number, string> = { 1: 'Sightings', 2: 'Friendships', 3: 'Mysteries' };

export const JOURNAL_KEY = 'agora-world-journal';

/** A single journal entry as the React Field Journal renders it. */
export interface JournalEntry {
  id: string;
  name: string;
  hint: string;
  found: boolean;
}

export interface JournalData {
  foundCount: number;
  total: number;
  percent: number;
  tiers: Array<{ tier: number; name: string; entries: JournalEntry[] }>;
}

export function journalData(state: EngineState): JournalData {
  const world = state.world as WorldState | null;
  const found = world?.found ?? {};
  const n = Object.keys(found).length;
  return {
    foundCount: n,
    total: EGGS.length,
    percent: Math.round(n / EGGS.length * 100),
    tiers: [1, 2, 3].map((tier) => ({
      tier,
      name: TIER_NAME[tier],
      entries: EGGS.filter((e) => e.tier === tier).map((e) => ({
        id: e.id,
        name: e.name,
        hint: e.hint,
        found: !!found[e.id],
      })),
    })),
  };
}

export interface SavedJournal {
  v: 1;
  found: Record<string, boolean>;
  unlocked: string[];
  firstLoad: number;
}

/**
 * Parse the persisted journal WITHOUT a live engine (used by the Field Guide
 * page, which renders even when the living background is off). Returns the
 * saved record, or null when nothing valid is stored.
 */
export function parseSavedJournal(raw: string | null): SavedJournal | null {
  if (!raw) return null;
  try {
    const d = JSON.parse(raw) as Partial<SavedJournal>;
    if (d.v !== 1 || typeof d.found !== 'object' || d.found === null) return null;
    return {
      v: 1,
      found: d.found,
      unlocked: Array.isArray(d.unlocked) ? d.unlocked : [],
      firstLoad: typeof d.firstLoad === 'number' ? d.firstLoad : 0,
    };
  } catch {
    return null;
  }
}

/** Build a JournalData read model from a saved record (no engine needed). */
export function journalFromSaved(saved: SavedJournal | null): JournalData {
  const found = saved?.found ?? {};
  const n = Object.keys(found).length;
  return {
    foundCount: n,
    total: EGGS.length,
    percent: Math.round(n / EGGS.length * 100),
    tiers: [1, 2, 3].map((tier) => ({
      tier,
      name: TIER_NAME[tier],
      entries: EGGS.filter((e) => e.tier === tier).map((e) => ({
        id: e.id,
        name: e.name,
        hint: e.hint,
        found: !!found[e.id],
      })),
    })),
  };
}

export function loadJournalState(state: EngineState, raw: string | null): SavedJournal | null {
  if (!raw) {
    state.firstLoad = Date.now();
    return null;
  }
  try {
    const d = JSON.parse(raw) as Partial<SavedJournal>;
    if (d.v !== 1) {
      state.firstLoad = Date.now();
      return null;
    }
    const world = state.world as WorldState | null;
    if (world) world.found = d.found ?? {};
    state.firstLoad = d.firstLoad || Date.now();
    return d as SavedJournal;
  } catch {
    state.firstLoad = Date.now();
    return null;
  }
}

export function saveJournalState(state: EngineState): string | null {
  const world = state.world as WorldState | null;
  try {
    return JSON.stringify({
      v: 1 as const,
      found: world?.found ?? {},
      unlocked: Object.keys(state.unlocked ?? {}),
      firstLoad: state.firstLoad,
    });
  } catch {
    return null;
  }
}

/** The milestone achievements (from checkAchievements). */
export interface AchievementDef {
  key: string;
  icon: string;
  name: string;
}

export function milestoneAchievements(state: EngineState, done: string[]): AchievementDef[] {
  const world = state.world as WorldState | null;
  const found = world?.found ?? {};
  const n = Object.keys(found).length;
  const out: AchievementDef[] = [];
  if (n >= 1) out.push({ key: 'first', icon: '🌟', name: 'First Discovery' });
  if (n >= 10) out.push({ key: 'nat', icon: '🦉', name: 'Naturalist' });
  if (n >= 25) out.push({ key: 'zoo', icon: '🦁', name: 'Zoologist' });
  if (n >= 40) out.push({ key: 'ranger', icon: '🎖️', name: 'Ranger' });
  if (n >= EGGS.length) out.push({ key: 'complete', icon: '👑', name: 'Completionist' });
  const t3 = EGGS.filter((e) => e.tier === 3);
  if (t3.length && t3.every((e) => found[e.id])) out.push({ key: 'puzzle', icon: '🧠', name: 'Puzzle Master' });
  const nightFound = NIGHT_EGG_IDS.filter((id) => found[id]).length;
  if (nightFound >= 5) out.push({ key: 'night', icon: '🌙', name: 'Night Owl' });
  if (state.firstLoad && (Date.now() - state.firstLoad) < 5 * 60 * 1000 && n >= 1) out.push({ key: 'quick', icon: '⏱️', name: 'Quick Study' });
  return out.filter((a) => done.indexOf(a.key) < 0);
}

/**
 * The bound `findEgg` — the prototype's global findEgg, wired to the engine's
 * event emitter. Marks the egg found, announces the discovery, runs the
 * milestone achievements, and (via the caller) persists the journal.
 */
export function makeFindEgg(state: EngineState, emit: (ev: import('./state').AmbienceEvent) => void): (id: string) => void {
  return (id: string) => {
    const world = state.world as WorldState | null;
    if (!world) return;
    const eg = EGG_BY_ID[id];
    if (!eg || world.found[id]) return;
    world.found[id] = true;
    emit({ type: 'discovery', eggId: id, eggName: eg.name, foundCount: Object.keys(world.found).length });
    checkAchievements(state, emit);
  };
}

/** Run the milestone achievements and emit any that are newly earned. */
export function checkAchievements(state: EngineState, emit: (ev: import('./state').AmbienceEvent) => void): void {
  const world = state.world as WorldState | null;
  if (!world) return;
  const fresh = milestoneAchievements(state, Object.keys(state.unlocked));
  fresh.forEach((a) => {
    state.unlocked[a.key] = true;
    emit({ type: 'achievement', icon: a.icon, name: a.name });
    if (a.key === 'complete') emit({ type: 'completion' });
  });
}
