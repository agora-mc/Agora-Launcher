/**
 * Ambience engine tests.
 *
 * The highest-value tests in the whole engine:
 *
 *  - terrain golden values for a fixed viewport — catches the F1
 *    `Math.floor`/`Math.round` regression permanently;
 *  - water span/hit alignment with the basin (F9);
 *  - the verify-eggs harness ported to vitest — drives all 54 easter eggs
 *    through their real handlers and must report 54/54 (F3/F10 included);
 *  - music: every voice sums to `beats`, opening frequencies match the score;
 *  - leak test: 50 mount/unmount cycles return the listener and rAF counts
 *    to baseline (trap 9).
 *
 * The world is built at module level (no canvas needed — the egg logic and
 * update loop never touch the DOM). `document.hidden` is false under jsdom,
 * and we drive frames manually with `world.update(1/60)` exactly as
 * verify-eggs.js documented.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from '@testing-library/react';
import { createEngineState, type EngineState } from './engine/state';
import { createWorld } from './engine/world';
import { regenerateTerrain, skyFrame } from './engine/sky';
import { advanceClock, createClock, type ClockState } from './engine/clock';
import { EGGS } from './engine/eggs';
import { groundYWorld, waterSpan } from './engine/terrain';
import { MUSIC_TRACKS } from './engine/audio/tracks';
import { SPECIES_BY_KEY } from './engine/species';
import { music, noteFreq } from './engine/audio/music';
import type { Species, WorldState } from './engine/types';
import { AmbienceCanvas } from './AmbienceCanvas';
import { AmbienceEngine } from './engine/engine';

/** The layout these tests were written against. */
const PINNED_WORLD_SEED = 20260811;

/* ── test world builder (no canvas) ─────────────────────────────────── */

interface TestWorld {
  state: EngineState;
  world: WorldState;
  clock: ClockState;
  events: unknown[];
}

function noop(): void { /* noop */ }

function buildTestWorld(): TestWorld {
  // Pin the world seed. The app seeds randomly per session so every launch is a
  // different place, but egg reachability is asserted against a KNOWN layout —
  // a random world would make this suite flaky for reasons unrelated to eggs.
  const state = createEngineState(false, PINNED_WORLD_SEED);
  state.ridgeSeeds = [1.3, 4.7, 8.1];
  // The terrain golden values below come from the prototype, which used these
  // three ridge seeds literally. The app now derives them from the session seed,
  // so pin them here to keep comparing against the same landscape.
  state.ridgeSeeds = [1.3, 4.7, 8.1];
  state.W = 1024;
  state.H = 768;
  const events: unknown[] = [];
  const world = createWorld(state, {
    blip: noop,
    burst: noop,
    emit: (ev) => { events.push(ev); },
  });
  state.world = world;
  // the engine regenerates terrain (and re-anchors the pond) on first frame
  regenerateTerrain(state);
  return { state, world, clock: createClock(), events };
}

/* verify-eggs harness helpers — ported verbatim from verify-eggs.js */

function makeHarness(t: TestWorld) {
  const { state, world: W } = t;

  function tick(seconds: number): number {
    const n = Math.round(seconds * 60);
    for (let i = 0; i < n; i++) W.update(1 / 60);
    return n;
  }
  const wait = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

  // Props are created once and mutate in place — restore every field a
  // reaction can touch, or earlier tests poison later ones.
  const PROP_SNAPSHOT: Array<{ p: WorldState['props'][number]; [k: string]: unknown }> = [];

  function snapshotProps() {
    PROP_SNAPSHOT.length = 0;
    W.props.forEach((p) => {
      PROP_SNAPSHOT.push({
        p,
        clicks: p.clicks, rolled: p.rolled, lit: p.lit, bloom: p.bloom,
        picked: p.picked, tipped: p.tipped, shrooms: p.shrooms, berries: p.berries,
        glow: p.glow, opened: p.opened, ripple: p.ripple, blink: p.blink,
      });
    });
  }

  function resetProps() {
    PROP_SNAPSHOT.forEach((s) => {
      const p = s.p;
      p.clicks = s.clicks; p.rolled = s.rolled; p.lit = s.lit; p.bloom = s.bloom;
      p.picked = s.picked; p.tipped = s.tipped; p.shrooms = s.shrooms; p.berries = s.berries;
      p.glow = s.glow; p.opened = s.opened; p.ripple = s.ripple; p.blink = s.blink;
    });
    // drop props created at runtime (snowman, rainbow ends, …)
    const keep = PROP_SNAPSHOT.map((s) => s.p);
    W.props = W.props.filter((p) => keep.indexOf(p) >= 0);
  }

  function reset() {
    W.found = {}; W.flags = {}; W.carry = null;
    W.entities.length = 0; W.items.length = 0;
    W.dl = 0.8; // default to daytime
    resetProps();
    // clock bounds are per-test state
    t.clock.fullDayMin = null; t.clock.fullDayMax = null;
    t.clock.acornMin = null; t.clock.acornMax = null;
    t.clock.weatherTimer = 0;
  }

  const props = (key: string) => W.props.filter((p) => p.key === key);
  function prop(key: string, n = 0) {
    const p = props(key)[n];
    if (p && p.reaction) p.reaction(p);
    return p;
  }
  function ent(key: string, opts?: { x?: number; fromLeft?: boolean }) {
    const species = SPECIES_BY_KEY[key];
    return species ? W.spawn(species, opts || { x: 600 }) : null;
  }
  function click(e: NonNullable<ReturnType<typeof ent>>) {
    W.interact({ kind: 'entity', obj: e });
  }
  function grab(id: string): boolean {
    const it = W.items.filter((i) => i.id === id)[0];
    if (!it) return false;
    W.interact({ kind: 'item', obj: it });
    return W.carry === id;
  }
  // F3: the fish is a clickable ENTITY arcing out of the pond — click it
  // mid-air to catch it (the real gameplay path).
  function catchFish(): boolean {
    prop('pond');
    const f = W.entities.filter((e) => e.sp && e.sp.key === 'fish')[0];
    if (!f) return false;
    tick(0.2);
    W.interact({ kind: 'entity', obj: f });
    return W.carry === 'fish';
  }
  const has = (id: string) => !!W.found[id];
  const night = () => { W.dl = 0.2; };
  const pondX = () => { const p = props('pond')[0]; return p ? p.x : 400; };

  // time/weather drivers (prototype's slider + weather button, now the clock)
  function sweepTime(): boolean {
    for (let v = 0; v <= 100; v += 5) {
      state.tod = v / 100;
      advanceClock(state, t.clock, 0, 0);
    }
    return true;
  }
  function rainThenClear(): void {
    // slot 1 = rain, then slot 2 = clear (during the day -> rainbow)
    t.clock.weatherTimer = 100;
    advanceClock(state, t.clock, 0.001, 0);
    t.clock.weatherTimer = 200;
    advanceClock(state, t.clock, 0.001, 0);
  }

  return { tick, wait, snapshotProps, reset, props, prop, ent, click, grab, catchFish, has, night, pondX, sweepTime, rainThenClear, W };
}

type SpeciesKey = Record<string, Species>;

/* ── terrain golden tests (F1) ──────────────────────────────────────── */

describe('terrain (port fidelity)', () => {
  let t: TestWorld;
  beforeEach(() => { t = buildTestWorld(); });

  it('regenerates ridges/basin/water for a fixed 1024×768 viewport', () => {
    expect(t.state.BASIN).toEqual({ c: 30, half: 11 });
    expect(t.state.WATER_LEVEL).toBeCloseTo(684.48, 1);
    expect(t.state.R1?.length).toBe(78);
    expect(t.state.R2?.length).toBeGreaterThan(80);
    expect(t.state.R3?.length).toBeGreaterThan(90);
  });

  it('groundYWorld matches the prototype golden values (F1 floor, not round)', () => {
    const cases: Array<[number, number, number]> = [
      [0, 16, 696.48],
      [50, 16, 660.48],
      [200, 16, 636.48],
      [555, 16, 660.48],
      [1000, 16, 648.48],
      [1100, 16, 624.48],
      [300, 14, 720.48],
      [900, 14, 696.48],
      [77, 12, 648.48],
      [500, 12, 648.48],
      [900, 12, 696.48],
    ];
    for (const [wx, layer, expected] of cases) {
      expect(groundYWorld(t.state, wx, layer)).toBeCloseTo(expected, 1);
    }
    // A point where Math.round would differ from Math.floor: (559+90)/16 =
    // 40.5625, so round would sample column 41 and floor samples 40. The
    // golden value below is the FLOOR value — the regression test that would
    // catch someone "fixing" the math back to Math.round.
    expect(groundYWorld(t.state, 559, 16)).toBeCloseTo(660.48, 1);
  });

  /**
   * A resize used to re-lay the whole landscape: the basin was carved at a
   * FRACTION of the ridge array (whose length follows the width), so the pond
   * slid sideways, and every ridge sat at a fraction of the height, so the
   * shoreline and the water level moved vertically too. Both are now anchored.
   */
  it('keeps the water where it is when the window is resized', () => {
    const span = waterSpan(t.state);
    // The shoreline's depth BELOW the bottom edge is the invariant — the world
    // is anchored there, so a taller window moves the absolute level down with
    // it rather than re-cutting the valley somewhere else.
    const depth = t.state.H - t.state.WATER_LEVEL;

    // wider and taller
    t.state.W = 1600; t.state.H = 900;
    regenerateTerrain(t.state);
    expect(waterSpan(t.state)).toEqual(span);
    expect(t.state.BASIN?.c).toBe(30);
    expect(t.state.H - t.state.WATER_LEVEL).toBeCloseTo(depth, 6);

    // narrower, still taller than the reference height
    t.state.W = 1200; t.state.H = 1000;
    regenerateTerrain(t.state);
    expect(waterSpan(t.state)).toEqual(span);
    expect(t.state.H - t.state.WATER_LEVEL).toBeCloseTo(depth, 6);

    // and back to where it started — the golden layout, unchanged
    t.state.W = 1024; t.state.H = 768;
    regenerateTerrain(t.state);
    expect(waterSpan(t.state)).toEqual(span);
    expect(t.state.WATER_LEVEL).toBeCloseTo(684.48, 1);
  });

  it('adds sky, not hills, when the window grows taller', () => {
    const groundAtLeftEdge = groundYWorld(t.state, 0, 2);
    const bottomGap = t.state.H - groundAtLeftEdge;

    t.state.H = 1100;
    regenerateTerrain(t.state);
    // the ground keeps its distance from the BOTTOM edge...
    expect(t.state.H - groundYWorld(t.state, 0, 2)).toBeCloseTo(bottomGap, 6);
    // ...so the extra height all went above the horizon.
    expect(groundYWorld(t.state, 0, 2)).toBeGreaterThan(groundAtLeftEdge);
  });

  it('falls back to proportional ridges below the reference height', () => {
    // Holding the offset on a much SHORTER window would push the hills off the
    // top of the screen; there the original H-fraction placement takes over.
    t.state.H = 400;
    regenerateTerrain(t.state);
    const ground = groundYWorld(t.state, 0, 2);
    expect(ground).toBeGreaterThan(0);
    expect(ground).toBeLessThan(400);
  });

  it('water sits inside the carved basin, not on a fixed ellipse (F9)', () => {
    const ws = waterSpan(t.state);
    expect(ws.x0).toBe(246);
    expect(ws.x1).toBe(318);
    // every submerged column is below the water level
    const b = t.state.BASIN!;
    const r3 = t.state.R3!;
    let submerged = 0;
    for (let i = b.c - b.half; i <= b.c + b.half; i++) {
      if (i < 0 || i >= r3.length) continue;
      if (r3[i] > t.state.WATER_LEVEL) submerged++;
    }
    expect(submerged).toBeGreaterThan(3);
  });

  it('skyFrame generates terrain on a fresh state (F-terrain regression)', () => {
    // The production bug: skyFrame set `state.lastW = state.W` BEFORE calling
    // regenerateTerrain, whose own `W !== lastW` guard then always early-
    // returned — the sky drew, but the terrain (ridges/basin/water) never
    // rendered. This test drives skyFrame directly on a fresh state and
    // asserts the ridges are actually generated.
    // Pin the world seed. The app seeds randomly per session so every launch is a
  // different place, but egg reachability is asserted against a KNOWN layout —
  // a random world would make this suite flaky for reasons unrelated to eggs.
  const state = createEngineState(false, PINNED_WORLD_SEED);
    state.W = 1024;
    state.H = 768;
    const world = createWorld(state, { blip: noop, burst: noop, emit: noop });
    state.world = world;
    // jsdom has no canvas; a minimal stub context is enough for skyFrame to
    // run (the terrain is generated before any drawing is consulted).
    const ctx = {
      createLinearGradient: () => ({ addColorStop: noop }),
      fillRect: noop,
      fillStyle: '',
      globalAlpha: 1,
      beginPath: noop,
      moveTo: noop,
      lineTo: noop,
      closePath: noop,
      fill: noop,
      stroke: noop,
      strokeStyle: '',
      lineWidth: 1,
      save: noop,
      restore: noop,
      translate: noop,
      scale: noop,
      arc: noop,
      ellipse: noop,
      clearRect: noop,
    } as unknown as CanvasRenderingContext2D;
    skyFrame(state, ctx, 0);
    expect(state.R1?.length).toBeGreaterThan(0);
    expect(state.R2?.length).toBeGreaterThan(0);
    expect(state.R3?.length).toBeGreaterThan(0);
    expect(state.BASIN).not.toBeNull();
    // And a second frame (no resize) does not wipe them.
    skyFrame(state, ctx, 16);
    expect(state.R3?.length).toBeGreaterThan(0);
  });
});

/* ── world clock: rainbow lifetime, time-of-day lock ────────────────── */

describe('world clock', () => {
  let t: TestWorld;
  let h: ReturnType<typeof makeHarness>;
  beforeEach(() => {
    t = buildTestWorld();
    h = makeHarness(t);
    h.snapshotProps();
    h.reset();
    t.world.dl = 0.8; // daytime, so rain -> clear puts a rainbow up
  });

  it('takes the weather rainbow down after a few seconds', () => {
    h.rainThenClear();
    expect(h.props('rainbow-end').length).toBe(2);
    h.tick(5);
    expect(h.props('rainbow-end').length).toBe(2);
    h.tick(6);
    expect(h.props('rainbow-end').length).toBe(0);
    expect(t.world.flags.rainbowUp).toBe(false);
  });

  it('leaves a pinned rainbow up indefinitely', () => {
    t.world.flags.rainbowPinned = true;
    t.world.spawnRainbow();
    h.tick(120);
    expect(h.props('rainbow-end').length).toBe(2);
  });

  it('re-raises a pinned rainbow after the egg claims the arc', () => {
    t.world.flags.rainbowPinned = true;
    t.world.spawnRainbow();
    h.props('rainbow-end').forEach((p) => p.reaction!(p));
    expect(h.has('rainbow')).toBe(true);
    h.tick(0.1);
    expect(h.props('rainbow-end').length).toBe(2);
  });

  it('holds the time of day while todLocked', () => {
    t.state.tod = 0.42;
    t.state.todLocked = true;
    advanceClock(t.state, t.clock, 60);
    expect(t.state.tod).toBe(0.42);
    t.state.todLocked = false;
    advanceClock(t.state, t.clock, 60);
    expect(t.state.tod).toBeGreaterThan(0.42);
  });

  it('holds the weather while weatherLocked', () => {
    t.state.weather = 2;
    t.state.weatherLocked = true;
    advanceClock(t.state, t.clock, 300);
    expect(t.state.weather).toBe(2);
  });
});

/* ── music data assertions ──────────────────────────────────────────── */

describe('music data (tracks.ts)', () => {
  it('has 13 verified tracks', () => {
    expect(MUSIC_TRACKS.length).toBe(13);
  });

  it('every voice sums to exactly the track beats (seam rule)', () => {
    for (const t of MUSIC_TRACKS) {
      for (const v of t.voices) {
        const got = v.seq.reduce((s, e) => s + e[1], 0);
        expect(Math.abs(got - t.beats)).toBeLessThan(0.001);
      }
    }
  });

  it('opening frequencies match the score (A4 = 440, middle C = C4)', () => {
    expect(noteFreq('A4')).toBeCloseTo(440, 6);
    expect(noteFreq('C4')).toBeCloseTo(261.6255653, 3);
    expect(noteFreq('A2')).toBeCloseTo(110, 6);
    // Mountain King opens on B3
    expect(noteFreq('B3')).toBeCloseTo(246.9416506, 3);
  });
});

/* ── autoplay track selection ─────────────────────────────────── */

describe('music autoplay ("let it choose")', () => {
  // The constructor only stashes the 2d context; jsdom has none and prints a
  // "not implemented" trace per canvas, which buries the real output.
  const originalGetContext = HTMLCanvasElement.prototype.getContext;
  beforeEach(() => {
    HTMLCanvasElement.prototype.getContext =
      (() => null) as typeof HTMLCanvasElement.prototype.getContext;
  });
  afterEach(() => { HTMLCanvasElement.prototype.getContext = originalGetContext; });

  /** No start(): `nextTrack` is pure bookkeeping and never touches a canvas. */
  function bareEngine(): AmbienceEngine {
    return new AmbienceEngine(
      document.createElement('canvas'),
      document.createElement('canvas'),
      { profile: 'calm', musicOn: false },
    );
  }

  /**
   * Deal n pieces the way autoplay does. Each pick becomes "now playing", which
   * is what the engine reads back to keep a bag from reopening on the piece the
   * last one closed with.
   */
  function deal(engine: AmbienceEngine, n: number): string[] {
    let playing: string | null = null;
    const spy = vi.spyOn(music, 'currentTrackId').mockImplementation(() => playing);
    const next = (engine as unknown as {
      nextTrack(all: typeof MUSIC_TRACKS): { id: string } | null;
    }).nextTrack.bind(engine);
    const out: string[] = [];
    for (let i = 0; i < n; i++) {
      const t = next(MUSIC_TRACKS);
      if (!t) throw new Error('autoplay ran dry');
      playing = t.id;
      out.push(t.id);
    }
    spy.mockRestore();
    return out;
  }

  it('draws from the whole library, not the mood of the hour', () => {
    // The mood filter this replaced could only ever reach 5 pieces by day and
    // 3 at night; Bumblebee and Fate were unreachable outright.
    const picks = deal(bareEngine(), MUSIC_TRACKS.length);
    expect(new Set(picks).size).toBe(MUSIC_TRACKS.length);
  });

  it('plays every piece once before any piece plays twice', () => {
    const rounds = 3;
    const picks = deal(bareEngine(), MUSIC_TRACKS.length * rounds);
    for (let r = 0; r < rounds; r++) {
      const bag = picks.slice(r * MUSIC_TRACKS.length, (r + 1) * MUSIC_TRACKS.length);
      expect(new Set(bag).size).toBe(MUSIC_TRACKS.length);
    }
  });

  it('never repeats back to back, bag seams included', () => {
    // A bag's one repeat risk is the seam. Unguarded it lands with p = 1/13 per
    // refill, so 40 runs x 2 seams would catch it ~99.8% of the time.
    for (let run = 0; run < 40; run++) {
      const picks = deal(bareEngine(), MUSIC_TRACKS.length * 3);
      for (let i = 1; i < picks.length; i++) {
        expect(picks[i]).not.toBe(picks[i - 1]);
      }
    }
  });

  /**
   * Stand in for the audio player. `start` is the only thing that can be
   * observed from outside the engine, so a spy on it is the whole record of
   * what autoplay decided to do.
   */
  function playerSpy(): { started: string[] } {
    // `music` is a module singleton, so a hook left behind by an earlier test
    // would let a "the hook is attached" assertion pass on its own.
    music.onPieceEnd = null;
    let playing: string | null = null;
    const started: string[] = [];
    vi.spyOn(music, 'start').mockImplementation((t) => {
      started.push(t.id);
      playing = t.id;
      return true;
    });
    vi.spyOn(music, 'currentTrackId').mockImplementation(() => playing);
    vi.spyOn(music, 'isPlaying').mockImplementation(() => playing !== null);
    return { started };
  }

  /** Let the engine's lazy `import('./audio/tracks')` settle. */
  const flush = () => new Promise((r) => { setTimeout(r, 0); });

  it('starts on a piece of its own choosing when music comes on', async () => {
    const engine = bareEngine();
    const player = playerSpy();
    engine.setMusicOn(true);
    await flush();
    expect(player.started).toHaveLength(1);
    expect(MUSIC_TRACKS.some((t) => t.id === player.started[0])).toBe(true);
    vi.restoreAllMocks();
  });

  it('still rotates at the end of a piece that was pinned by hand', async () => {
    // Pinning starts playback without going through setMusicOn, which is where
    // the end-of-piece hook used to be installed -- so autoplay went deaf for
    // the rest of the session the first time anybody picked a piece.
    const engine = bareEngine();
    const player = playerSpy();
    engine.setTrack(MUSIC_TRACKS[0].id);
    await flush();
    expect(player.started).toEqual([MUSIC_TRACKS[0].id]);
    expect(music.onPieceEnd).not.toBeNull();

    engine.setMusicAuto(true);
    music.onPieceEnd?.();
    expect(player.started).toHaveLength(2);
    expect(player.started[1]).not.toBe(player.started[0]);
    vi.restoreAllMocks();
  });

  it('moves off the pinned piece the moment "let it choose" is picked', async () => {
    // Repeated because the fresh bag still holds the pinned id: roughly one run
    // in MUSIC_TRACKS.length draws it first and has to draw again.
    for (let run = 0; run < 40; run++) {
      const engine = bareEngine();
      const player = playerSpy();
      engine.setTrack(MUSIC_TRACKS[0].id);
      await flush();
      engine.shuffleNow();
      await flush();
      expect(engine.musicAuto).toBe(true);
      expect(player.started).toHaveLength(2);
      expect(player.started[1]).not.toBe(MUSIC_TRACKS[0].id);
      vi.restoreAllMocks();
    }
  });

  it('leaves silence alone: "let it choose" is a running order, not a play button', async () => {
    const engine = bareEngine();
    const player = playerSpy();
    engine.shuffleNow();
    await flush();
    expect(player.started).toEqual([]);
    expect(engine.musicAuto).toBe(true);
    vi.restoreAllMocks();
  });
});

/* ── the 54-egg harness (verify-eggs port) ──────────────────────────── */

describe('verify-eggs (all 54 discoveries reachable)', () => {
  it('drives every egg through its real handler and reports 54/54', async () => {
    const t = buildTestWorld();
    const h = makeHarness(t);
    h.snapshotProps();

    const CREATURE_CLICK: Array<[string, string]> = [
      ['fox-nap', 'fox'], ['deer-stare', 'deer'], ['bear-roar', 'bear'], ['bird-song', 'songbird'],
      ['goose-honk', 'goose'], ['gull-scream', 'seagull'], ['hog-ball', 'hedgehog'], ['wolf-howl', 'wolf'],
      ['squirrel-nut', 'squirrel'], ['rabbit-thump', 'rabbit'], ['owl-spin', 'owl'], ['bat-loop', 'bat'],
      ['frog-jump', 'frog'], ['turtle-hide', 'turtle'], ['butterfly-land', 'butterfly'],
      ['crow-scatter', 'crow'], ['pecker-hole', 'woodpecker'], ['raccoon-guilt', 'raccoon'],
      ['moose-bellow', 'moose'], ['boar-truffle', 'boar'], ['mouse-hide', 'mouse'],
    ];

    const TESTS: Array<[string, () => boolean | string | Promise<boolean | string>]> = [];

    CREATURE_CLICK.forEach((pair) => {
      TESTS.push([pair[0], () => {
        h.night(); // night species are eligible; day ones ignore it
        const e = h.ent(pair[1]);
        if (!e) return `species "${pair[1]}" missing`;
        h.tick(0.2); // uids are assigned in the update loop
        h.click(e);
        return h.has(pair[0]) || 'clicked, no findEgg';
      }]);
    });

    TESTS.push(
      ['firefly-sync', () => {
        h.night();
        const f = [];
        for (let i = 0; i < 3; i++) f.push(h.ent('firefly', { x: 300 + i * 40 }));
        h.tick(0.2);
        f.forEach((e) => e && h.click(e));
        return h.has('firefly-sync') || 'needs 3 distinct fireflies';
      }],
      ['flower-bloom', () => { h.prop('flowers'); return h.has('flower-bloom') || 'no fire'; }],
      ['rock-beetle', () => { h.prop('rock'); return h.has('rock-beetle') || 'no fire'; }],
      ['log-shroom', () => { h.prop('log'); return h.has('log-shroom') || 'no fire'; }],
      ['pond-fish', () => {
        h.prop('pond');
        const f = t.world.entities.filter((e) => e.sp && e.sp.key === 'fish')[0];
        if (!f) return 'no fish spawned';
        h.tick(0.2);
        h.click(f);
        return h.has('pond-fish') || 'catching the fish did not fire';
      }],

      /* --- tier 2 --- */
      ['acorn-squirrel', () => {
        h.prop('oak'); if (!h.grab('acorn')) return 'oak dropped no acorn';
        const s = h.ent('squirrel'); h.tick(0.2); h.click(s!);
        return h.has('acorn-squirrel') || 'squirrel did not accept acorn';
      }],
      ['pinecone-squirrel', () => {
        h.prop('pine'); if (!h.grab('pinecone')) return 'pine dropped no pinecone';
        const s = h.ent('squirrel'); h.tick(0.2); h.click(s!);
        return h.has('pinecone-squirrel') || 'squirrel did not accept pinecone';
      }],
      ['berry-hog', () => {
        h.prop('bush'); if (!h.grab('berry')) return 'bush yielded no berry';
        const hg = h.ent('hedgehog'); h.tick(0.2); h.click(hg!);
        return h.has('berry-hog') || 'hedgehog did not accept berry';
      }],
      ['honey-bear', () => {
        const hv = h.props('hive')[0];
        if (!hv) return 'no hive prop';
        hv.reaction!(hv); hv.reaction!(hv); hv.reaction!(hv);
        if (!h.grab('honey')) return 'hive yielded no honey after 3 clicks';
        const b = h.ent('bear'); h.tick(0.2); h.click(b!);
        return h.has('honey-bear') || 'bear did not accept honey';
      }],
      ['feather-scarecrow', () => {
        const g = h.ent('seagull'); h.tick(0.2); h.click(g!);
        if (!h.grab('feather')) return 'seagull dropped no feather';
        h.prop('scarecrow');
        return h.has('feather-scarecrow') || 'scarecrow did not accept feather';
      }],
      ['truffle-pond', () => {
        const b = h.ent('boar'); h.tick(0.2); h.click(b!);
        if (!h.grab('truffle')) return 'boar yielded no truffle';
        t.world.drop(h.pondX(), 500);
        return h.has('truffle-pond') || 'pond did not accept truffle';
      }],
      ['acorn-pond', () => {
        h.prop('oak'); if (!h.grab('acorn')) return 'no acorn';
        t.world.drop(h.pondX(), 500);
        return h.has('acorn-pond') || 'pond did not accept acorn';
      }],
      ['duck-line', () => {
        const d = h.ent('duck'); h.tick(0.2);
        for (let i = 0; i < 6; i++) h.click(d!);
        return h.has('duck-line') || '6 clicks did not line up ducklings';
      }],
      ['boulder-hole', () => {
        const b = h.props('boulder')[0];
        if (!b) return 'no boulder prop';
        for (let i = 0; i < 6; i++) b.reaction!(b);
        return h.has('boulder-hole') || 'boulder did not roll';
      }],
      ['bee-hive', () => {
        for (let i = 0; i < 4; i++) {
          const bee = h.ent('bee', { x: 300 + i * 50 });
          h.tick(0.2);
          h.click(bee!);
        }
        return h.has('bee-hive') || 'following bees did not reveal the hive';
      }],
      ['snail-lily', () => {
        // F13: a snail clicked at the pond's edge fires directly
        const s = h.ent('snail', { x: h.pondX() }); h.tick(0.2); h.click(s!);
        return h.has('snail-lily') || 'snail near pond did not fire';
      }],
      ['fish-otter', () => {
        const o = h.ent('otter'); h.tick(0.2);
        if (!h.catchFish()) return 'BLOCKED: cannot catch a fish (F3)';
        h.click(o!);
        return h.has('fish-otter') || 'otter did not accept fish';
      }],
      ['flower-deer', () => {
        h.props('flowers').forEach((p) => p.reaction!(p));
        if (!h.grab('flower')) return 'BLOCKED: nothing produces a flower item (F10)';
        const d = h.ent('deer'); h.tick(0.2); h.click(d!);
        return h.has('flower-deer') || 'deer did not accept flower';
      }],
      ['firefly-cave', () => {
        h.night();
        const f = h.ent('firefly'); h.tick(0.2); h.click(f!);
        if (t.world.carry !== 'firefly') return 'BLOCKED: firefly is not carryable (F10)';
        h.prop('cave');
        return h.has('firefly-cave') || 'cave did not accept firefly';
      }],

      /* --- tier 3 --- */
      ['moonlit-rave', () => {
        h.night();
        const f = [];
        for (let i = 0; i < 7; i++) f.push(h.ent('firefly', { x: 300 + i * 40 }));
        h.tick(0.2);
        f.forEach((e) => e && h.click(e));
        const sw = t.world.entities.filter((e) => e.data && e.data.isSwarm)[0];
        if (!sw) return '7 fireflies did not form a swarm';
        h.tick(0.2);
        h.click(sw);
        return h.has('moonlit-rave') || 'swarm click did not fire';
      }],
      ['migration', () => {
        const g = h.ent('goose', { x: 600 }); h.tick(0.2);
        h.click(g!); h.tick(0.3); h.click(g!); h.tick(0.3); h.click(g!);
        return h.has('migration') || 'three lead-goose clicks did not fire';
      }],
      ['wolf-pack', () => {
        h.night();
        const w = h.ent('wolf', { x: 600 }); h.tick(0.2); h.click(w!); h.tick(0.3);
        const others = t.world.entities.filter((e) => e.sp === (SPECIES_BY_KEY as SpeciesKey).wolf && e !== w);
        others.forEach((e) => h.click(e));
        return h.has('wolf-pack') || `only ${others.length} wolves answered`;
      }],
      ['snowman', () => {
        const piles = h.props('snowpile');
        if (!piles.length) return 'no snowpile props';
        piles.slice().sort((a, b) => (a.rank as number) - (b.rank as number))
          .forEach((p) => p.reaction!(p));
        const sm = h.props('snowman')[0];
        if (!sm) return 'piles clicked in size order did not build a snowman';
        sm.reaction!(sm);
        return h.has('snowman') || 'snowman click did not fire';
      }],
      ['constellation', () => {
        h.night();
        const st = h.props('star');
        if (!st.length) return 'no star props';
        [5, 4, 3, 2, 1].forEach((r) => {
          const s = st.filter((x) => x.rank === r)[0];
          if (s) s.reaction!(s);
        });
        return h.has('constellation') || 'brightness order did not fire';
      }],
      ['fairy-ring', () => {
        const ms = h.props('mushroom');
        if (ms.length < 7) return `only ${ms.length} mushrooms in the ring`;
        ms.forEach((m) => m.reaction!(m));
        return h.has('fairy-ring') || 'all mushrooms within the window did not fire';
      }],
      ['rainbow', () => {
        t.world.dl = 0.8; // must be daytime
        h.rainThenClear();
        const ends = h.props('rainbow-end');
        if (!ends.length) return 'rain->clear in daylight spawned no rainbow';
        ends.forEach((e) => e.reaction!(e));
        return h.has('rainbow') || 'clicking both ends did not fire';
      }],
      ['acorn-hunt', () => {
        // counts acorns DROPPED at 3 separate spots clear of the pond
        const px = h.pondX();
        const spots: number[] = [];
        [0.2, 0.5, 0.8].forEach((f) => {
          let s = Math.round(1024 * f);
          if (Math.abs(s - px) < 150) s = s < px ? Math.max(20, s - 300) : Math.min(1004, s + 300);
          spots.push(s);
        });
        spots.forEach((x) => {
          h.prop('oak');
          if (h.grab('acorn')) t.world.drop(x, 500);
        });
        if (!t.world.flags.acornHuntArmed) return 'three buries did not arm the hunt';
        h.sweepTime();
        return h.has('acorn-hunt') || 'armed, but a full day sweep did not fire';
      }],
      ['full-day', () => {
        h.sweepTime();
        return h.has('full-day') || 'full sweep did not fire';
      }],
      ['water-flowers', () => {
        // flowers consume the carried water (one dose per patch); the player
        // refills from the pond between patches — the intended loop
        const ok = h.props('flowers').every((p) => {
          h.prop('pond'); // refill (empty-handed each time)
          if (t.world.carry !== 'water') return false;
          p.reaction!(p);
          return true;
        });
        if (!ok) return 'BLOCKED: nothing produces a water item (F10)';
        return h.has('water-flowers') || 'watering all patches did not fire';
      }],
      ['bear-fish', () => {
        const b = h.ent('bear'); h.tick(0.2);
        if (!h.catchFish()) return 'BLOCKED: cannot catch a fish (F3)';
        h.click(b!);
        return h.has('bear-fish') || 'bear did not accept fish';
      }],
      ['bear-feast', () => {
        const b = h.ent('bear'); h.tick(0.2);
        if (!h.catchFish()) return 'BLOCKED: cannot catch a fish (F3)';
        h.click(b!);
        ['berry', 'honey'].forEach((i) => { t.world.carry = i; h.click(b!); });
        return h.has('bear-feast') || 'three courses did not fire';
      }],
      ['the-long-con', () => {
        const bo = h.props('boulder')[0];
        for (let i = 0; i < 6; i++) bo.reaction!(bo);
        if (!t.world.flags.boulderMoved) return 'boulder did not roll';
        const fx = h.ent('fox', { x: 700 }); h.tick(0.2);
        if (!h.catchFish()) return 'BLOCKED: cannot catch a fish (F3)';
        h.click(fx!); // befriend
        if (!fx!.data.friend) return 'fox not befriended by fish';
        h.tick(1.5);
        h.click(fx!); // send to fetch
        if (!fx!.data.fetchQuest) return 'second click did not start the fetch';
        h.tick(20);
        return h.wait(1200).then(() => { // key drops on a real 900ms timer
          const key = t.world.items.filter((i) => i.id === 'key')[0];
          if (!key) return 'fox fetched nothing';
          t.world.interact({ kind: 'item', obj: key });
          if (t.world.carry !== 'key') return 'key not picked up';
          h.prop('cave');
          return h.has('the-long-con') || 'cave did not open with the key';
        });
      }],
      ['campfire-tales', () => {
        // real setTimeouts (~4.8s) — cannot be fast-forwarded
        h.night();
        const cf = h.props('campfire')[0];
        if (!cf) return 'no campfire prop';
        cf.reaction!(cf); cf.reaction!(cf); cf.reaction!(cf);
        if (!cf.lit) return 'three clicks did not light the fire';
        return h.wait(5600).then(() => {
          const g = t.world.entities.filter((e) => e.data && e.data.atCampfire);
          if (g.length < 3) return `only ${g.length} animals gathered`;
          g.forEach((e) => h.click(e));
          return h.has('campfire-tales') || 'clicking all three did not fire';
        });
      }],
    );

    const results: Array<{ egg: string; ok: boolean; detail: string }> = [];
    const failures: string[] = [];

    for (const [id, fn] of TESTS) {
      h.reset();
      let out: boolean | string;
      try {
        const maybe = fn();
        if (maybe && typeof (maybe as Promise<boolean | string>).then === 'function') {
          out = await (maybe as unknown as Promise<boolean | string>);
        } else {
          out = maybe as boolean | string;
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        out = 'ERROR: ' + message;
      }
      const ok = out === true;
      results.push({ egg: id, ok, detail: ok ? '' : String(out) });
      if (!ok) failures.push(`${id} — ${String(out)}`);
    }

    h.reset();
    const passed = results.filter((r) => r.ok).length;
    // Registry may declare more eggs than the harness covers; the harness must
    // cover every declared egg, and all of those must pass.
    const declared = EGGS.map((e) => e.id);
    const covered = results.map((r) => r.egg);
    const uncovered = declared.filter((d) => covered.indexOf(d) < 0);
    expect(uncovered).toEqual([]);
    expect(passed).toBe(EGGS.length);
    expect(failures).toEqual([]);
  }, 30000);
});

/* ── leak test (trap 9) ─────────────────────────────────────────────── */

describe('AmbienceCanvas lifecycle', () => {
  // jsdom has no 2d canvas context; give the engine a permissive stub.
  const originalGetContext = HTMLCanvasElement.prototype.getContext;
  let rafCalls = 0;
  let cafCalls = 0;

  beforeEach(() => {
    const makeCtx = () => {
      const target: Record<string, unknown> = {};
      return new Proxy(target, {
        get(t, prop) {
          if (prop === 'canvas') return { width: 0, height: 0 };
          if (prop in t) return t[prop as string];
          if (prop === 'createLinearGradient' || prop === 'createRadialGradient' || prop === 'createPattern') {
            return () => ({ addColorStop: () => undefined });
          }
          if (prop === 'measureText') return () => ({ width: 0 });
          if (prop === 'getImageData') return () => ({ data: [] });
          return () => undefined;
        },
        set(t, prop, val) { t[prop as string] = val; return true; },
      });
    };
    HTMLCanvasElement.prototype.getContext = function (contextId: string) {
      if (contextId !== '2d') return null;
      return makeCtx() as unknown as CanvasRenderingContext2D;
    } as typeof HTMLCanvasElement.prototype.getContext;
    rafCalls = 0;
    cafCalls = 0;
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => { rafCalls++; return 1; });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => { cafCalls++; });
  });

  afterEach(() => {
    HTMLCanvasElement.prototype.getContext = originalGetContext;
    vi.restoreAllMocks();
  });

  it('releases every listener and rAF handle across 50 mount/unmount cycles', () => {
    // Scope the spies to `window` and `document` only — the engine attaches to
    // exactly those (resize/load/pointermove on window; click/visibility on
    // document). React's own root listeners go on a container div and never
    // reach these targets, so the net must be zero per target.
    const winAdd = vi.spyOn(window, 'addEventListener');
    const winRem = vi.spyOn(window, 'removeEventListener');
    const docAdd = vi.spyOn(document, 'addEventListener');
    const docRem = vi.spyOn(document, 'removeEventListener');

    // One warmup cycle settles any one-time environment listener (jsdom and
    // React attach a single document listener on first render); from there the
    // engine must be leak-free: each further cycle adds and removes the same
    // listeners, so the net over the remaining 50 cycles is exactly zero.
    const warmup = render(
      <AmbienceCanvas
        profile="calm"
        soundOn={false}
        musicVolume={0.35}
        musicOn={false}
        reducedMotion={false}
        onEvent={() => undefined}
      />,
    );
    warmup.unmount();

    const base = {
      winAdd: winAdd.mock.calls.length,
      winRem: winRem.mock.calls.length,
      docAdd: docAdd.mock.calls.length,
      docRem: docRem.mock.calls.length,
    };

    for (let i = 0; i < 50; i++) {
      const { unmount } = render(
        <AmbienceCanvas
          profile="calm"
          soundOn={false}
          musicVolume={0.35}
          musicOn={false}
          reducedMotion={false}
          onEvent={() => undefined}
        />,
      );
      unmount();
    }

    // listeners added === listeners removed (net zero) on each target
    expect(winAdd.mock.calls.length - base.winAdd).toBe(winRem.mock.calls.length - base.winRem);
    expect(docAdd.mock.calls.length - base.docAdd).toBe(docRem.mock.calls.length - base.docRem);
    // rAF scheduled in mount and cancelled in unmount (net zero)
    expect(rafCalls).toBe(cafCalls);
  });
});

/* ── host scoping (Stage 1: boxed-world options) ───────────────────── */

/**
 * The engine's `host`/`getBounds` options fit the world to a boxed container
 * (the web hero diorama) instead of the window. The leak contract must hold
 * there too: a host scopes pointer listeners to itself, and the ResizeObserver
 * it creates has to be disconnected in stop(), or every mount/unmount cycle
 * with a host would leak an observer. jsdom has no ResizeObserver, so these
 * tests install a recorder and drive the engine directly (AmbienceCanvas does
 * not forward host/getBounds — the desktop component stays untouched).
 */
describe('AmbienceEngine host scoping', () => {
  const originalGetContext = HTMLCanvasElement.prototype.getContext;
  let rafCalls = 0;
  let cafCalls = 0;
  let observed: Element[] = [];
  let disconnects = 0;

  beforeEach(() => {
    const makeCtx = () => {
      const target: Record<string, unknown> = {};
      return new Proxy(target, {
        get(t, prop) {
          if (prop === 'canvas') return { width: 0, height: 0 };
          if (prop in t) return t[prop as string];
          if (prop === 'createLinearGradient' || prop === 'createRadialGradient' || prop === 'createPattern') {
            return () => ({ addColorStop: () => undefined });
          }
          if (prop === 'measureText') return () => ({ width: 0 });
          if (prop === 'getImageData') return () => ({ data: [] });
          return () => undefined;
        },
        set(t, prop, val) { t[prop as string] = val; return true; },
      });
    };
    HTMLCanvasElement.prototype.getContext = function (contextId: string) {
      if (contextId !== '2d') return null;
      return makeCtx() as unknown as CanvasRenderingContext2D;
    } as typeof HTMLCanvasElement.prototype.getContext;
    rafCalls = 0;
    cafCalls = 0;
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => { rafCalls++; return 1; });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => { cafCalls++; });
    observed = [];
    disconnects = 0;
    class FakeResizeObserver {
      constructor(public cb: ResizeObserverCallback) {}
      observe(target: Element) { observed.push(target); }
      unobserve() { /* noop */ }
      disconnect() { disconnects++; }
    }
    (globalThis as Record<string, unknown>).ResizeObserver = FakeResizeObserver;
  });

  afterEach(() => {
    HTMLCanvasElement.prototype.getContext = originalGetContext;
    vi.restoreAllMocks();
    delete (globalThis as Record<string, unknown>).ResizeObserver;
  });

  function mountEngine(host?: HTMLElement): AmbienceEngine {
    const bg = document.createElement('canvas');
    const fx = document.createElement('canvas');
    const engine = new AmbienceEngine(bg, fx, {
      profile: 'calm',
      musicOn: false,
      reducedMotion: false,
      host,
      getBounds: host
        ? () => {
            const r = host.getBoundingClientRect();
            return { left: r.left, top: r.top, w: r.width, h: r.height };
          }
        : undefined,
    });
    engine.start();
    return engine;
  }

  it('creates no ResizeObserver without a host (desktop default untouched)', () => {
    const engine = mountEngine();
    expect(observed).toHaveLength(0);
    engine.stop();
    expect(disconnects).toBe(0);
  });

  it('observes the host on start and disconnects it on stop', () => {
    const host = document.createElement('div');
    const engine = mountEngine(host);
    expect(observed).toEqual([host]);
    engine.stop();
    expect(disconnects).toBe(1);
  });

  it('stays leak-free across 50 start/stop cycles with a host', () => {
    const host = document.createElement('div');
    const winAdd = vi.spyOn(window, 'addEventListener');
    const winRem = vi.spyOn(window, 'removeEventListener');
    const docAdd = vi.spyOn(document, 'addEventListener');
    const docRem = vi.spyOn(document, 'removeEventListener');
    const hostAdd = vi.spyOn(host, 'addEventListener');
    const hostRem = vi.spyOn(host, 'removeEventListener');

    // One warmup cycle settles environment listeners; from there the net on
    // every target (window, document, and the host itself) must be zero.
    const warmup = mountEngine(host);
    warmup.stop();
    const base = {
      winAdd: winAdd.mock.calls.length,
      winRem: winRem.mock.calls.length,
      docAdd: docAdd.mock.calls.length,
      docRem: docRem.mock.calls.length,
      hostAdd: hostAdd.mock.calls.length,
      hostRem: hostRem.mock.calls.length,
    };

    for (let i = 0; i < 50; i++) {
      const engine = mountEngine(host);
      engine.stop();
    }

    expect(winAdd.mock.calls.length - base.winAdd).toBe(winRem.mock.calls.length - base.winRem);
    expect(docAdd.mock.calls.length - base.docAdd).toBe(docRem.mock.calls.length - base.docRem);
    expect(hostAdd.mock.calls.length - base.hostAdd).toBe(hostRem.mock.calls.length - base.hostRem);
    // Every cycle created exactly one observer and stop() disconnected it.
    expect(observed.length).toBe(disconnects);
    // rAF scheduled in start and cancelled in stop (net zero).
    expect(rafCalls).toBe(cafCalls);
  });
});
