/**
 * Shared mutable state for the ambience engine.
 *
 * This is the TypeScript home of the globals the v4-world prototype kept in
 * one IIFE scope (W/H, mouse, time-of-day, weather, ridges, basin, world).
 * Keeping them in ONE mutable object is deliberate: the port rule is 1:1
 * fidelity, and threading dozens of parameters through every function would
 * reshape the code. The engine owns one instance; every engine module reads
 * and mutates it exactly as the prototype's globals were read and mutated.
 *
 * The prototype comment that must survive this port, verbatim in intent:
 *   "innerWidth can still be 0 on the first tick in an embedded frame; fall
 *    back and re-measure on load so the backdrop never stays a 0x0 buffer."
 */

export type WeatherId = 'clear' | 'rain' | 'snow';

/**
 * Events the engine emits for the React shell to present (toasts, the Field
 * Journal, the carry tag). The engine logic never touches the DOM for these.
 */
export type AmbienceEvent =
  | { type: 'discovery'; eggId: string; eggName: string; foundCount: number }
  | { type: 'achievement'; icon: string; name: string }
  | { type: 'completion' }
  | { type: 'carry'; itemId: string; name: string; firstTime: boolean }
  | { type: 'drop-carry' };

/** One raindrop / snowflake / firefly / star, as in the prototype. */
export interface Drop { x: number; y: number; s: number; }
export interface Flake { x: number; y: number; s: number; p: number; }
export interface Firefly { x: number; y: number; p: number; s: number; }
export interface Star { x: number; y: number; p: number; r: number; }

/** The carved bowl that holds the pond (F9). */
export interface Basin { c: number; half: number; }

/** Reduced-motion flag: cached from matchMedia at engine construction. */
let cachedReduce = false;
if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
  cachedReduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}
export function reduceMotion(): boolean {
  return cachedReduce;
}

export interface EngineState {
  W: number;
  H: number;
  /** Time of day, 0..1 (0 = deep night, .5 = noon). */
  tod: number;
  /** Index into WEATHER. */
  weather: number;
  WEATHER: WeatherId[];
  /** Lightning flash overlay strength. */
  flash: number;
  /** Mouse position in 0..1 space. */
  mx: number;
  my: number;
  reduce: boolean;
  drops: Drop[];
  flakes: Flake[];
  fireflies: Firefly[];
  stars: Star[];
  R1: number[] | null;
  R2: number[] | null;
  R3: number[] | null;
  lastW: number;
  WATER_LEVEL: number;
  BASIN: Basin | null;
  /** Terrain is overscanned past both edges by OVER px (see ridge). */
  OVER: number;
  /** The living world; null until the engine builds it. */
  world: unknown;
  lastTs: number;
  /** Prototype SFX mute (independent of music). */
  soundOn: boolean;
  /** Earned achievements (name-keyed, for the toast/journal). */
  unlocked: Record<string, boolean>;
  /** First-load timestamp, used by the "Quick Study" achievement. */
  firstLoad?: number;
  /** Seeded PRNG — fixed seed: same world every load, so it's learnable. */
  wrand: () => number;
  /** Timestamp of the last shelf drag (prototype's `justDragged` guard). */
  justDragged?: number;
  /** Weather scheduler accumulator. */
  weatherTimer?: number;
}

export function createEngineState(reduce = reduceMotion()): EngineState {
  const state: EngineState = {
    W: 0,
    H: 0,
    tod: 0.3,
    weather: 0,
    WEATHER: ['clear', 'rain', 'snow'],
    flash: 0,
    mx: 0.5,
    my: 0.5,
    reduce,
    drops: [],
    flakes: [],
    fireflies: [],
    stars: [],
    R1: null,
    R2: null,
    R3: null,
    lastW: 0,
    WATER_LEVEL: 0,
    BASIN: null,
    OVER: 90,
    world: null,
    lastTs: 0,
    soundOn: false,
    unlocked: {},
    wrand: mulberry32(20260811),
  };
  // --- prototype initialisation, verbatim ---
  for (let i = 0; i < 220; i++) state.drops.push({ x: Math.random(), y: Math.random(), s: 0.5 + Math.random() });
  for (let i = 0; i < 150; i++) state.flakes.push({ x: Math.random(), y: Math.random(), s: 0.4 + Math.random(), p: Math.random() * 6.3 });
  for (let i = 0; i < 26; i++) state.fireflies.push({ x: Math.random(), y: 0.55 + Math.random() * 0.4, p: Math.random() * 6.3, s: 0.3 + Math.random() });
  for (let i = 0; i < 130; i++) state.stars.push({ x: Math.random(), y: Math.random() * 0.55, p: Math.random() * 6.3, r: Math.random() < 0.12 ? 2 : 1 });
  return state;
}

/** Prototype helpers, verbatim. */
export function lerp(a: number, b: number, k: number): number { return a + (b - a) * k; }
export function rgba(c: number[], a: number): string {
  return 'rgba(' + (c[0] | 0) + ',' + (c[1] | 0) + ',' + (c[2] | 0) + ',' + a + ')';
}
export function mixc(a: number[], b: number[], k: number): number[] {
  return [lerp(a[0], b[0], k), lerp(a[1], b[1], k), lerp(a[2], b[2], k)];
}

/**
 * Seeded PRNG — fixed seed: same world every load, so it's learnable.
 * Mulberry32, verbatim from the prototype.
 */
export function mulberry32(seed: number): () => number {
  return function () {
    seed |= 0; seed = seed + 0x6D2B79F5 | 0;
    const t = Math.imul(seed ^ seed >>> 15, 1 | seed);
    const t2 = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t;
    return ((t2 ^ t2 >>> 14) >>> 0) / 4294967296;
  };
}
