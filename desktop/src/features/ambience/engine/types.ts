/**
 * Shared types for the ambience engine.
 *
 * Everything here is type-only (no runtime code) so that `world.ts`,
 * `species.ts`, `props.ts`, `eggs.ts` and `api.ts` can import each other's
 * types without creating runtime import cycles.
 */

import type { VoiceDef } from './api';

export interface Palette {
  body: string;
  accent?: string;
  dark: string;
  belly?: string;
  eye?: string;
}

export interface ShapeDef {
  bw?: number; bh?: number; hw?: number; hh?: number;
  lw?: number; lh?: number; tw?: number; th?: number;
  ew?: number; eh?: number;
  ears?: boolean; tail?: boolean; legs?: boolean;
  earShape?: 'point' | 'round' | 'long' | 'none';
  tailShape?: 'stub' | 'bushy' | 'long' | 'flat' | 'none';
  snout?: number; neck?: number; hump?: number;
  antlers?: boolean; tusks?: boolean; spikes?: boolean; shell?: boolean;
  mane?: boolean; mask?: boolean;
}

export interface Species {
  key: string;
  name: string;
  /** Spawn weight (0 = gated, spawned only when its gate is open). */
  w: number;
  /** D = day, N = night, A = any. */
  t: 'D' | 'N' | 'A';
  layer: number | 'sky' | 'pond';
  layerRange?: number[];
  speed: number;
  size?: number;
  kind: 'bird' | 'quad' | 'flutter' | 'glow' | 'fish';
  group?: boolean;
  perch?: boolean;
  swim?: boolean;
  shape?: ShapeDef;
  pal: Palette;
  voice?: VoiceDef;
  /** F11: gated species (w:0 + gate) spawn directly when the gate opens. */
  gate?: () => boolean;
  accepts?: string[];
  onClick?: (e: Entity) => void;
  behave?: (e: Entity, dt: number) => void;
}

export interface Entity {
  sp: Species;
  layer: number | 'sky' | 'pond';
  dir: number;
  vx: number;
  scale: number;
  state: string;
  t: number;
  stateT: number;
  phase: number;
  fx: string | null;
  data: Record<string, unknown>;
  hb: { w: number; h: number };
  removeAt: number | null;
  /** WORLD x (F7): transformed to screen only at draw/hit time. */
  x: number;
  /** WORLD y — only meaningful for sky (and fish arcs via jy). */
  y: number;
  uid?: string;
  /** F8a: pond & perch residents get a lifespan so they don't crowd out. */
  residentUntil?: number;
  sleepStart?: number;
  sleepFor?: number;
  /** How long a react state lasts (set by reactState). */
  reactDur?: number;
  jy?: number;
  vy?: number;
  waterY?: number;
  pondCX?: number;
  pondR?: number;
  groupMembers?: Entity[];
  groupOffset?: number;
}

export interface Prop {
  key: string;
  name: string;
  layer: number | 'sky' | 'pond';
  /** WORLD x (F7). */
  x: number;
  y?: number;
  /** Some props (the pond) have no blocky w/h — they draw from hb/rw. */
  w?: number;
  h?: number;
  hb?: { w: number; h: number };
  visibleIf?: () => boolean;
  reaction?: (p: Prop) => void;
  /** Per-prop state lives in extra fields (hole, shakeT, bloom, …). */
  [k: string]: unknown;
}

export interface WorldItem {
  id: string;
  /** WORLD x (F7). */
  x: number;
  y: number;
  t: number;
}

export interface WorldFx {
  x: number;
  y: number;
  text: string;
  life: number;
  vy: number;
}

export interface HitResult {
  kind: 'item' | 'entity' | 'prop';
  obj: WorldItem | Entity | Prop;
}

export interface SpawnOptions {
  x?: number;
  fromLeft?: boolean;
  /** Extra entity data (e.g. the migration chain). */
  data?: Record<string, unknown>;
}

/** A carryable item picked up by the player (for the React carry tag). */
export interface LastPickup {
  itemId: string;
  name: string;
  firstTime: boolean;
  at: number;
}

export interface WorldState {
  t: number;
  entities: Entity[];
  props: Prop[];
  items: WorldItem[];
  carry: string | null;
  flags: Record<string, unknown>;
  found: Record<string, boolean>;
  hover: HitResult | null;
  hoverIsProp: boolean;
  dl: number;
  spawnTimer: number;
  sinceLastSpawn: number;
  worldFx: WorldFx[];
  firstLoad?: number;
  lastInteractAt?: number;
  _pond?: Prop;
  /** Last picked-up carryable (for the React carry tag). */
  lastPickup?: LastPickup;
  /** Event emitter wired by the engine (toasts, journal, sounds). */
  emit?: (ev: import('./state').AmbienceEvent) => void;
  /** Discovery lookup bound after the world is built (prototype's global `findEgg`). */
  _findEgg?: (id: string) => void;

  layerScale(layer: number | 'sky' | 'pond'): number;
  isNight(): boolean;
  isDay(): boolean;
  eligible(sp: Species): boolean;
  spawn(sp: Species, opts?: SpawnOptions): Entity;
  pondProp(): Prop | undefined;
  despawn(e: Entity): void;
  hit(px: number, py: number): HitResult | null;
  nearestProp(x: number, key: string): Prop | null;
  nearPond(x: number): boolean;
  pet(species: string): void;
  dropCarry(): void;
  pickUp(itemId: string, x: number, y: number): void;
  spawnFish(pond: Prop): Entity;
  splash(x: number, y: number): void;
  spawnRainbow(): void;
  checkRainbow(): void;
  /** Take the arc down and reset its lifetime (fade-out, or the egg claiming it). */
  clearRainbow(): void;
  spawnSwarm(): void;
  campfireClick(e: Entity, key: string): void;
  startCampfireTales(fire: Prop): void;
  ringHit(idx: number): void;
  update(dt: number): void;
  popFx(x: number, y: number, text: string): void;
  drawEntity(ctx: CanvasRenderingContext2D, e: Entity): void;
  drawProp(ctx: CanvasRenderingContext2D, p: Prop): void;
  drawItem(ctx: CanvasRenderingContext2D, it: WorldItem): void;
  drawHoverGlow(ctx: CanvasRenderingContext2D, hit: HitResult): void;
  draw(ctx: CanvasRenderingContext2D): void;
  interact(hit: HitResult): void;
  drop(x: number, y: number): void;
}
