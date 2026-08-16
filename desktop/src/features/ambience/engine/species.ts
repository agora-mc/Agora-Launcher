/**
 * The 30-species wildlife catalogue + the procedural blocky critter renderers.
 *
 * The state helpers (reactState, fleeState, boltState, exitToward,
 * dropGroundItem, playVoice, findEgg, shakeProp) come from the bound GameApi.
 */

import type { GameApi } from './api';
import type { Entity, Palette, ShapeDef, Species } from './types';

function darken(hex: string, amt: number): string {
  hex = hex.replace('#', '');
  const r = parseInt(hex.substr(0, 2), 16), g = parseInt(hex.substr(2, 2), 16), b = parseInt(hex.substr(4, 2), 16);
  return 'rgb(' + Math.max(0, r - amt) + ',' + Math.max(0, g - amt) + ',' + Math.max(0, b - amt) + ')';
}

export function P(body: string, accent?: string, belly?: string): Palette {
  return { body: body, accent: accent || body, dark: darken(body, 55), belly: belly, eye: '#161616' };
}

const DEFSHAPE: ShapeDef = {
  bw: 16, bh: 10, hw: 8, hh: 8, lw: 3, lh: 6, tw: 8, th: 6, ew: 3, eh: 5, ears: true, tail: true, legs: true,
  // --- silhouette features (F4): every quadruped shared one box outline before this ---
  earShape: 'point', // point | round | long | none
  tailShape: 'stub', // stub | bushy | long | flat | none
  snout: 0,         // muzzle projection, px
  neck: 0,          // how far the head sits above the body
  hump: 0,          // shoulder hump height
  antlers: false, tusks: false, spikes: false, shell: false, mane: false, mask: false,
};

export function shape(o: ShapeDef): ShapeDef {
  const s: ShapeDef = {};
  for (const k in DEFSHAPE) (s as Record<string, unknown>)[k] = (DEFSHAPE as Record<string, unknown>)[k];
  for (const k2 in o) (s as Record<string, unknown>)[k2] = (o as Record<string, unknown>)[k2];
  return s;
}

/**
 * Procedural blocky critter rendering (fillRect only, no images/paths).
 * Ported verbatim — including the F2 four-leg fix (hips fixed to the body
 * underside, feet lift) and the F4 per-species silhouettes.
 */
export function drawQuad(
  ctx: CanvasRenderingContext2D, x: number, gy: number, s: number, dir: number,
  pal: Palette, phase: number, walking: boolean, shapeDef?: ShapeDef, squash?: number,
): { w: number; h: number } {
  shapeDef = shapeDef || {}; squash = squash || 1;
  const bw = (shapeDef.bw || 16) * s, bh = (shapeDef.bh || 10) * s * squash;
  const hw = (shapeDef.hw || 8) * s, hh = (shapeDef.hh || 8) * s;
  const lw = Math.max(1, (shapeDef.lw || 3) * s), lh = (shapeDef.lh || 6) * s;
  const by = gy - lh - bh;
  const bx0 = x - bw / 2;
  const R = Math.round;
  const F = (fx: number, fy: number, fw: number, fh: number) => {
    ctx.fillRect(R(fx), R(fy), Math.max(1, R(fw)), Math.max(1, R(fh)));
  };
  // --- legs: hip fixed to the body underside, foot lifts (F2). Four legs, diagonal gait. ---
  if (shapeDef.legs !== false) {
    const hipY = gy - lh;
    const liftA = walking ? Math.max(0, Math.sin(phase)) * lh * 0.5 : 0;
    const liftB = walking ? Math.max(0, Math.sin(phase + Math.PI)) * lh * 0.5 : 0;
    ctx.fillStyle = pal.dark;
    F(bx0 + bw * 0.10, hipY, lw, lh - liftA);
    F(bx0 + bw * 0.26, hipY, lw, lh - liftB);
    F(bx0 + bw * 0.62, hipY, lw, lh - liftB);
    F(bx0 + bw * 0.80, hipY, lw, lh - liftA);
  }
  // --- tail: shape-specific, behind the body ---
  const tailK = shapeDef.tailShape || 'stub';
  if (shapeDef.tail !== false && tailK !== 'none') {
    const tw = (shapeDef.tw || 8) * s, th = (shapeDef.th || 6) * s;
    const back = dir > 0 ? bx0 - tw * 0.7 : bx0 + bw - tw * 0.3; // rear edge, whichever way it faces
    ctx.fillStyle = pal.accent || pal.body;
    if (tailK === 'bushy') {                                  // fox / squirrel: widening plume, curls up
      for (let q = 0; q < 3; q++) F(back + (dir > 0 ? -q * 2 * s : q * 2 * s), by + bh * 0.1 - q * 2.2 * s, tw * 0.55 + q * 1.6 * s, th * 0.62);
    } else if (tailK === 'long') {                            // wolf: low straight sweep
      F(back + (dir > 0 ? -tw * 0.35 : tw * 0.35), by + bh * 0.34, tw * 1.15, th * 0.36);
    } else if (tailK === 'flat') {                            // beaver-ish paddle
      F(back, by + bh * 0.42, tw * 1.05, th * 0.85);
    } else {                                                  // stub
      F(back + (dir > 0 ? tw * 0.25 : -tw * 0.25), by + bh * 0.12, tw * 0.5, th * 0.55);
    }
  }
  // --- shoulder hump (bear, boar): stacked steps above the front of the back ---
  if (shapeDef.hump) {
    const hpx = dir > 0 ? bx0 + bw * 0.46 : bx0 + bw * 0.1, hstep = shapeDef.hump * s / 3;
    ctx.fillStyle = pal.body;
    for (let hI = 0; hI < 3; hI++) F(hpx + hI * 1.2 * s, by - hstep * (3 - hI), bw * 0.42 - hI * 1.6 * s, hstep * (3 - hI) + 1);
  }
  // --- body ---
  ctx.fillStyle = pal.body;
  F(bx0, by, bw, bh);
  if (pal.belly) { ctx.fillStyle = pal.belly; F(bx0, by + bh * 0.55, bw, bh * 0.42); }
  // --- spikes along the back (hedgehog) ---
  if (shapeDef.spikes) {
    ctx.fillStyle = pal.dark;
    for (let sI = 0; sI < 9; sI++) F(bx0 + bw * (0.04 + sI * 0.108), by - 5.2 * s - (sI % 2) * 1.4 * s, 1.5 * s, 6.4 * s + (sI % 2) * 1.4 * s);
  }
  // --- shell dome (turtle), drawn over the body ---
  if (shapeDef.shell) {
    ctx.fillStyle = pal.accent || pal.dark;
    F(bx0 - bw * 0.10, by - bh * 0.70, bw * 1.20, bh * 0.95);
    F(bx0 + bw * 0.10, by - bh * 1.10, bw * 0.80, bh * 0.5);
    F(bx0 + bw * 0.28, by - bh * 1.35, bw * 0.44, bh * 0.35);
    ctx.fillStyle = pal.dark;
    for (let kI = 0; kI < 3; kI++) F(bx0 + bw * (0.22 + kI * 0.24), by - bh * 0.45, 1.4 * s, bh * 0.5);
  }
  // --- head, raised by neck ---
  const neck = (shapeDef.neck || 0) * s;
  const hx = bx0 + (dir > 0 ? bw - hw * 0.35 : -hw * 0.65);
  const hy = by - hh * 0.45 - neck;
  if (neck) {                                               // visible throat column linking body to head
    ctx.fillStyle = pal.body;
    F(hx + (dir > 0 ? -hw * 0.05 : hw * 0.42), hy + hh * 0.55, hw * 0.6, neck + hh * 0.5);
  }
  if (shapeDef.mane) {                                      // wolf ruff between head and shoulders
    ctx.fillStyle = pal.dark;
    F(hx + (dir > 0 ? -hw * 0.42 : hw * 0.5), hy + hh * 0.1, hw * 0.62, hh * 1.05);
  }
  ctx.fillStyle = pal.body;
  F(hx, hy, hw, hh);
  if (shapeDef.mask && pal.belly) {                         // raccoon/badger face band
    ctx.fillStyle = pal.dark; F(hx, hy + hh * 0.28, hw, hh * 0.3);
  }
  // --- snout ---
  if (shapeDef.snout) {
    const sn = shapeDef.snout * s;
    ctx.fillStyle = pal.belly || pal.dark;
    F(dir > 0 ? hx + hw * 0.82 : hx - sn * 0.9, hy + hh * 0.42, sn, hh * 0.42);
  }
  // --- tusks (boar) ---
  if (shapeDef.tusks) {
    ctx.fillStyle = '#F2EBD8';
    const tx0 = dir > 0 ? hx + hw * 0.95 : hx - hw * 0.2;
    F(tx0, hy + hh * 0.5, 1.6 * s, 3.4 * s);
    F(tx0 + (dir > 0 ? -1.8 * s : 1.8 * s), hy + hh * 0.34, 1.4 * s, 2.4 * s);
  }
  // --- antlers (deer, moose) ---
  if (shapeDef.antlers) {
    ctx.fillStyle = pal.dark;
    [0.18, 0.62].forEach((off) => {
      const ax = hx + hw * off;
      F(ax, hy - hh * 0.95, 1.6 * s, hh * 1.0);                    // main beam
      F(ax + (dir > 0 ? 2.2 * s : -2.2 * s), hy - hh * 0.6, 2.6 * s, 1.5 * s); // lower branch
      F(ax + (dir > 0 ? 3.0 * s : -3.0 * s), hy - hh * 1.0, 2.2 * s, 1.5 * s); // upper branch
    });
  }
  // --- ears ---
  const earK = shapeDef.earShape || (shapeDef.ears === false ? 'none' : 'point');
  if (shapeDef.ears !== false && earK !== 'none') {
    const ew = (shapeDef.ew || 3) * s, eh = (shapeDef.eh || 5) * s;
    ctx.fillStyle = pal.dark;
    if (earK === 'long') {                                    // rabbit / hare
      F(hx + hw * 0.14, hy - eh * 1.9, ew * 0.85, eh * 2.0);
      F(hx + hw * 0.55, hy - eh * 1.7, ew * 0.85, eh * 1.85);
    } else if (earK === 'round') {                            // bear / mouse / raccoon
      F(hx + hw * 0.02, hy - eh * 0.6, ew * 1.5, eh * 0.85);
      F(hx + hw * 0.6, hy - eh * 0.6, ew * 1.5, eh * 0.85);
    } else {                                                  // pointed
      F(hx + hw * 0.08, hy - eh * 0.85, ew, eh);
      F(hx + hw * 0.58, hy - eh * 0.85, ew, eh);
    }
  }
  ctx.fillStyle = pal.eye || '#161616';
  F(hx + (dir > 0 ? hw * 0.62 : hw * 0.14), hy + hh * 0.32, s, s);
  return { w: bw + hw, h: bh + hh + lh + neck + (shapeDef.antlers ? hh : 0) };
}

export function drawBird(
  ctx: CanvasRenderingContext2D, x: number, y: number, s: number, dir: number,
  pal: Palette, phase: number, flying: boolean,
): { w: number; h: number } {
  const bw = 10 * s, bh = 7 * s;
  const flap = flying ? Math.sin(phase * 10) * 4 * s : Math.sin(phase * 2) * 1 * s;
  ctx.fillStyle = pal.accent || pal.body;
  ctx.fillRect(Math.round(x - bw * 0.1), Math.round(y - flap), Math.round(bw * 0.9), Math.max(1, Math.round(3 * s)));
  ctx.fillRect(Math.round(x - bw * 0.6), Math.round(y + flap * 0.6), Math.round(bw * 0.9), Math.max(1, Math.round(3 * s)));
  ctx.fillStyle = pal.body;
  ctx.fillRect(Math.round(x - bw * 0.35), Math.round(y - bh * 0.3), Math.round(bw * 0.7), Math.round(bh * 0.6));
  const hx = x + (dir > 0 ? bw * 0.3 : -bw * 0.55);
  ctx.fillRect(Math.round(hx), Math.round(y - bh * 0.55), Math.round(bw * 0.4), Math.round(bh * 0.4));
  ctx.fillStyle = pal.dark;
  ctx.fillRect(Math.round(hx + (dir > 0 ? bw * 0.4 : -bw * 0.12)), Math.round(y - bh * 0.42), Math.max(1, Math.round(2 * s)), Math.max(1, Math.round(1.5 * s)));
  return { w: bw * 1.6, h: bh * 1.6 };
}

export function drawFlutter(
  ctx: CanvasRenderingContext2D, x: number, y: number, s: number, phase: number, pal: Palette,
): { w: number; h: number } {
  const w = 6 * s, bob = Math.sin(phase * 3) * 3 * s;
  const wing = Math.abs(Math.sin(phase * 9)) * w;
  ctx.fillStyle = pal.accent || pal.body;
  ctx.fillRect(Math.round(x - wing), Math.round(y + bob - w * 0.4), Math.round(wing), Math.round(w * 0.8));
  ctx.fillRect(Math.round(x), Math.round(y + bob - w * 0.4), Math.round(wing), Math.round(w * 0.8));
  ctx.fillStyle = pal.body;
  ctx.fillRect(Math.round(x - s), Math.round(y + bob - s), Math.round(2 * s), Math.round(2 * s));
  return { w: w * 2, h: w };
}

export const SPECIES_BY_KEY: Record<string, Species> = {};

/** The 30-species catalogue, filled by buildSpecies. */
export const SPECIES: Species[] = [];

/**
 * Build the 30-species catalogue with the game API bound so the onClick
 * handlers keep their prototype bodies. Replaces the registry each call so a
 * fresh world (a new engine, or a test rebuild) gets handlers bound to it.
 */
export function buildSpecies(api: GameApi): Species[] {
  const W = api.world;
  const { reactState, boltState, exitToward, dropGroundItem, playVoice, findEgg } = api;

  const ALL: Species[] = [
    {
      key: 'songbird', name: 'Songbird', w: 10, t: 'D', layer: 'sky', speed: 34, kind: 'bird', pal: P('#E6533E', '#F5A64B'),
      voice: { freqs: [880, 1046, 1318], gap: 60 }, onClick: function (e) { reactState(e, 'chirp', 0.6); e.vy = -40; playVoice(this.voice); findEgg('bird-song'); },
    },
    {
      key: 'goose', name: 'Canada goose', w: 5, t: 'D', layer: 'sky', speed: 52, kind: 'bird', group: true, pal: P('#5B5145', '#2A241C'),
      voice: { freqs: [220, 196], gap: 90 }, onClick: function (e) {
        const g = (e.groupMembers as Entity[] | undefined) || [e];
        g.forEach(function (m, i) {
          setTimeout(function () { reactState(m, 'honk', 0.5); api.blip(220 - i * 6, 0.16, 'sawtooth', 0.09); }, i * 140);
        });
        findEgg('goose-honk');
        if (e === g[0] && !W().found['migration']) {
          const f = W().flags;
          f.migrationStep = ((f.migrationStep as number) || 0) + 1;
          const step = f.migrationStep as number;
          if (step === 1) { g.forEach(function (m) { m.state = 'idle'; m.stateT = 99; }); }
          else if (step === 2) { g.forEach(function (m) { m.state = 'idle'; m.stateT = 99; m.y += (2 - g.indexOf(m) % 2); }); }
          else if (step >= 3) {
            findEgg('migration');
            g.forEach(function (m) { m.state = 'walk'; m.vx = (Math.random() < 0.5 ? -1 : 1) * 60; });
          }
        }
      },
    },
    {
      key: 'seagull', name: 'Seagull', w: 6, t: 'D', layer: 'sky', speed: 30, kind: 'bird', pal: P('#E9EDF0', '#B9C4CC'),
      voice: { freqs: [1400, 1200], gap: 80 }, onClick: function (e) { reactState(e, 'screech', 0.6); playVoice(this.voice);
        dropGroundItem('feather', e.x); findEgg('gull-scream'); },
    },
    {
      key: 'deer', name: 'Deer', w: 8, t: 'D', layer: 2, layerRange: [1, 2], speed: 22, kind: 'quad',
      shape: shape({ bw: 18, bh: 12, hw: 8, hh: 9, lh: 9, lw: 2, neck: 6, antlers: true, earShape: 'long', eh: 4, tailShape: 'stub' }),
      pal: P('#A6693B', '#7A4A26', '#EAD9BE'),
      voice: { freqs: [520, 700] }, onClick: function (e) {
        if (e.data.atCampfire) { W().campfireClick(e, 'deer'); return; }
        if (W().carry === 'flower' && !e.data.tame) { e.data.tame = true; W().dropCarry(); reactState(e, 'pet', 1.2); playVoice({ freqs: [660, 880, 1100], gap: 90 }); findEgg('flower-deer'); return; }
        if (e.data.tame) { reactState(e, 'pet', 1); W().pet('deer'); return; }
        reactState(e, 'stare', 2); setTimeout(function () { if (e.state === 'react') boltState(e); }, 1900); playVoice(this.voice); findEgg('deer-stare');
      },
    },
    {
      key: 'bear', name: 'Bear', w: 4, t: 'D', layer: 2, layerRange: [0, 1], speed: 14, size: 1.5, kind: 'quad',
      shape: shape({ bw: 26, bh: 16, hw: 12, hh: 11, lh: 8, lw: 5, ears: true, tail: false, earShape: 'round', hump: 4, snout: 3 }),
      pal: P('#5B4433', '#3C2E22'),
      voice: { freqs: [110, 140], t: 'sawtooth', gap: 120 }, onClick: function (e) {
        const carried = W().carry;
        if (carried && (this.accepts as string[]).indexOf(carried) >= 0) {
          const fed = (e.data.fed = (e.data.fed as Record<string, boolean>) || {});
          fed[carried] = true; W().dropCarry();
          reactState(e, 'happy', 1.3); playVoice({ freqs: [440, 554, 659, 880], gap: 80 });
          if (carried === 'fish') findEgg('bear-fish');
          if (carried === 'honey') findEgg('honey-bear');
          const have = e.data.fed as Record<string, boolean>;
          if (have.fish && have.berry && have.honey) findEgg('bear-feast');
          return;
        }
        reactState(e, 'roar', 1.1); playVoice(this.voice); findEgg('bear-roar');
      }, accepts: ['fish', 'berry', 'honey'],
    },
    {
      key: 'fox', name: 'Fox', w: 8, t: 'A', layer: 2, layerRange: [1, 2], speed: 26, kind: 'quad',
      shape: shape({ bw: 16, bh: 9, tailShape: 'bushy', tw: 10, earShape: 'point', eh: 6, snout: 4 }),
      pal: P('#E2761B', '#F5A64B', '#FFF3E0'),
      voice: { freqs: [700, 900, 1200], gap: 60 }, onClick: function (e) {
        if (e.data.atCampfire) { W().campfireClick(e, 'fox'); return; }
        const carried = W().carry;
        if (carried === 'fish' && !e.data.friend) { e.data.friend = true; W().dropCarry(); reactState(e, 'friend', 1.2);
          playVoice({ freqs: [600, 800, 1000], gap: 70 }); W().flags.foxFriend = true; return; }
        if (e.data.friend && !e.data.fetched && W().flags.boulderMoved) {
          e.state = 'exit'; e.data.exitTarget = W().flags.boulderX; e.dir = (e.data.exitTarget as number) > e.x ? 1 : -1;
          e.vx = Math.abs(e.vx) * e.dir * 1.4;
          e.data.fetchQuest = true; return;
        }
        reactState(e, 'curl', 2.6); playVoice(this.voice); findEgg('fox-nap');
      },
    },
    {
      key: 'wolf', name: 'Wolf', w: 4, t: 'N', layer: 2, layerRange: [1, 2], speed: 24, kind: 'quad',
      shape: shape({ bw: 18, bh: 9, mane: true, earShape: 'point', snout: 5, tailShape: 'long', tw: 11, lh: 10, lw: 2 }),
      pal: P('#6B6B70', '#48484D'),
      voice: { freqs: [300, 240, 180], gap: 180, d: 0.4 }, onClick: function (e) {
        if (e.data.isAnswer) {
          reactState(e, 'howl', 1); playVoice({ freqs: [280, 220], gap: 150, d: 0.3 });
          if (!e.data.answered) {
            e.data.answered = true;
            const n = ((W().flags.packAnswered as number) || 0) + 1;
            W().flags.packAnswered = n;
            if (n >= 3 && !W().found['wolf-pack']) {
              findEgg('wolf-pack');
              W().entities.forEach(function (x) { if (x.sp === SPECIES_BY_KEY.wolf) boltState(x); });
            }
          }
          return;
        }
        reactState(e, 'howl', 1.4); playVoice(this.voice); findEgg('wolf-howl');
        if (!W().flags.packSpawned) {
          W().flags.packSpawned = true; W().flags.packAnswered = 0;
          for (let i = 0; i < 3; i++) {
            const w = W().spawn(SPECIES_BY_KEY.wolf, { x: e.x + (i + 1) * 90 * (e.dir || 1), fromLeft: e.dir > 0 });
            w.data.isAnswer = true;
          }
        }
      },
    },
    {
      key: 'squirrel', name: 'Squirrel', w: 9, t: 'D', layer: 2, layerRange: [1, 2], speed: 44, kind: 'quad',
      shape: shape({ bw: 11, bh: 7, hw: 6, hh: 6, lh: 4, lw: 2, tw: 7, th: 9, tailShape: 'bushy', earShape: 'point', eh: 4 }),
      pal: P('#B5652B', '#8A4A1E'),
      voice: { freqs: [1200, 1500, 1200], gap: 50 }, onClick: function (e) {
        const carried = W().carry;
        if (carried && (carried === 'acorn' || carried === 'pinecone')) {
          W().dropCarry(); reactState(e, 'bury', 1);
          playVoice({ freqs: [900, 1100], gap: 60 });
          if (carried === 'acorn') findEgg('acorn-squirrel'); else findEgg('pinecone-squirrel'); return;
        }
        reactState(e, 'nut', 1); dropGroundItem('acorn', e.x); playVoice(this.voice); findEgg('squirrel-nut');
      },
    },
    {
      key: 'hedgehog', name: 'Hedgehog', w: 5, t: 'A', layer: 2, speed: 12, kind: 'quad',
      shape: shape({ bw: 12, bh: 8, ears: false, tail: false, spikes: true, snout: 3, lh: 3 }),
      pal: P('#8A7A63', '#5E5142'),
      voice: { freqs: [500, 400], gap: 100 }, onClick: function (e) {
        const carried = W().carry;
        if (carried === 'berry' && !e.data.friend) {
          e.data.friend = true; W().dropCarry(); reactState(e, 'happy', 1);
          playVoice({ freqs: [600, 800], gap: 80 }); findEgg('berry-hog'); return;
        }
        reactState(e, 'ball', 1.4); playVoice(this.voice); findEgg('hog-ball');
      },
    },
    {
      key: 'rabbit', name: 'Rabbit', w: 9, t: 'A', layer: 2, layerRange: [1, 2], speed: 50, kind: 'quad',
      shape: shape({ bw: 10, bh: 9, hw: 6, hh: 6, lh: 5, lw: 2, ears: true, eh: 6, earShape: 'long', tailShape: 'stub', tw: 4, th: 4 }),
      pal: P('#C8B79A', '#9C8C70', '#F2E9DA'),
      voice: { freqs: [900, 900], gap: 120 }, onClick: function (e) {
        if (e.data.atCampfire) { W().campfireClick(e, 'rabbit'); return; }
        reactState(e, 'thump', 0.8); playVoice(this.voice);
        setTimeout(function () { if (e.state !== 'gone') boltState(e); }, 750); findEgg('rabbit-thump');
      },
    },
    {
      key: 'owl', name: 'Owl', w: 6, t: 'N', layer: 1, speed: 0, kind: 'bird', perch: true, pal: P('#7C6A4E', '#584B37'),
      voice: { freqs: [300, 240], gap: 260, d: 0.3 }, onClick: function (e) { reactState(e, 'rotate', 1.4); playVoice(this.voice); findEgg('owl-spin'); },
    },
    {
      key: 'bat', name: 'Bat', w: 6, t: 'N', layer: 'sky', speed: 60, kind: 'bird', pal: P('#3C3440', '#241E29'),
      voice: { freqs: [2200, 2600], gap: 40, d: 0.05 }, onClick: function (e) { reactState(e, 'loop', 1); playVoice(this.voice); findEgg('bat-loop'); },
    },
    {
      key: 'frog', name: 'Frog', w: 5, t: 'A', layer: 'pond', speed: 0, kind: 'quad',
      shape: shape({ bw: 10, bh: 7, hw: 8, hh: 6, lh: 3, lw: 2, ears: false, tail: false, snout: 2 }),
      pal: P('#5E9E4A', '#3D7031', '#CFE7A0'),
      voice: { freqs: [180], t: 'square', d: 0.12 }, onClick: function (e) { reactState(e, 'jump', 0.7); playVoice(this.voice);
        e.data.jumpTo = e.x + (Math.random() < 0.5 ? -1 : 1) * 60; findEgg('frog-jump'); },
    },
    {
      key: 'duck', name: 'Duck', w: 5, t: 'D', layer: 'pond', speed: 10, kind: 'bird', swim: true, pal: P('#4A7B3B', '#2E5426'),
      voice: { freqs: [520, 470], gap: 100 }, onClick: function (e) { reactState(e, 'quack', 0.6); playVoice(this.voice);
        const n = ((W().flags.duckClicks as number) || 0) + 1;
        W().flags.duckClicks = n;
        if (n >= 5) { e.data.line = true; findEgg('duck-line'); } },
    },
    {
      key: 'turtle', name: 'Turtle', w: 3, t: 'D', layer: 2, speed: 6, kind: 'quad',
      shape: shape({ bw: 14, bh: 9, ears: false, lh: 3, shell: true, neck: 3, tailShape: 'none' }),
      pal: P('#4E7A4A', '#345234', '#8FC77A'),
      voice: { freqs: [220], t: 'square' }, onClick: function (e) { reactState(e, 'shell', 3); playVoice(this.voice); findEgg('turtle-hide'); },
    },
    {
      key: 'butterfly', name: 'Butterfly', w: 4, t: 'D', layer: 2, speed: 20, kind: 'flutter', pal: P('#E6659E', '#F2A6C6'),
      voice: { freqs: [1400, 1700], gap: 70 }, onClick: function (e) { reactState(e, 'land', 8); playVoice(this.voice); findEgg('butterfly-land'); },
    },
    {
      key: 'firefly', name: 'Firefly', w: 4, t: 'N', layer: 2, speed: 8, kind: 'glow', pal: P('#CFFF7A', '#CFFF7A'),
      voice: { freqs: [1800], d: 0.08 }, onClick: function (e) {
        reactState(e, 'glow', 1.2); playVoice(this.voice);
        const fc = (W().flags.fireflyClicks = (W().flags.fireflyClicks as Record<string, boolean>) || {});
        fc[e.uid as string] = true;
        const n = Object.keys(fc).length;
        if (n >= 3 && !W().found['firefly-sync']) findEgg('firefly-sync');
        if (W().isNight() && n >= 7 && !W().flags.swarmSpawned) { W().flags.swarmSpawned = true; W().spawnSwarm(); }
        // F10: an empty-handed click lets you carry one (there is no ground item)
        if (!W().carry) { e.state = 'gone'; W().pickUp('firefly', e.x, e.y); }
      },
    },
    {
      key: 'crow', name: 'Crow', w: 5, t: 'D', layer: 1, speed: 0, kind: 'bird', perch: true, pal: P('#26242A', '#141318'),
      voice: { freqs: [300], t: 'sawtooth', d: 0.18 }, onClick: function (e) {
        reactState(e, 'caw', 0.7); playVoice(this.voice);
        for (let i = 0; i < 3; i++) {
          const c = W().spawn(SPECIES_BY_KEY.crow, { x: e.x, fromLeft: Math.random() < 0.5 });
          c.layer = 1; c.state = 'flee'; c.vx = (Math.random() < 0.5 ? -1 : 1) * (90 + i * 20);
        }
        findEgg('crow-scatter');
      },
    },
    {
      key: 'woodpecker', name: 'Woodpecker', w: 4, t: 'D', layer: 2, speed: 0, kind: 'bird', perch: true, pal: P('#D0392B', '#1C1C1C'),
      voice: { freqs: [1000, 1000, 1000], gap: 40, d: 0.03 }, onClick: function (e) {
        reactState(e, 'peck', 1); playVoice(this.voice);
        const oak = W().nearestProp(e.x, 'oak'); if (oak) oak.hole = true; findEgg('pecker-hole');
      },
    },
    {
      key: 'raccoon', name: 'Raccoon', w: 4, t: 'N', layer: 2, speed: 24, kind: 'quad',
      shape: shape({ bw: 13, bh: 8, lh: 3, lw: 2, tw: 9, th: 6, tailShape: 'bushy', earShape: 'round', ew: 4, eh: 4, mask: true, snout: 3 }),
      pal: P('#7C7A78', '#4E4C4A', '#D8D5CE'),
      voice: { freqs: [500, 600, 500], gap: 60 }, onClick: function (e) {
        reactState(e, 'guilty', 0.8); playVoice(this.voice);
        setTimeout(function () { if (e.state !== 'gone') boltState(e); }, 750); findEgg('raccoon-guilt');
      },
    },
    {
      key: 'moose', name: 'Moose', w: 1, t: 'D', layer: 1, layerRange: [0, 1], speed: 10, size: 1.7, kind: 'quad',
      shape: shape({ bw: 30, bh: 19, hw: 14, hh: 12, lh: 10, lw: 4, tail: false, antlers: true, neck: 7, hump: 5, snout: 5, earShape: 'long', eh: 4 }),
      pal: P('#4A3B2E', '#2E241B'),
      voice: { freqs: [90], t: 'sawtooth', d: 0.6 }, onClick: function (e) { reactState(e, 'bellow', 1.2); playVoice(this.voice); findEgg('moose-bellow'); },
    },
    {
      key: 'boar', name: 'Boar', w: 3, t: 'A', layer: 2, layerRange: [0, 1], speed: 20, kind: 'quad',
      shape: shape({ bw: 16, bh: 10, ears: true, earShape: 'round', ew: 2, eh: 3, tusks: true, snout: 5, hump: 3, lh: 4, tailShape: 'stub', tw: 4, th: 3 }),
      pal: P('#4A4038', '#302A24'),
      voice: { freqs: [160, 200], t: 'sawtooth' }, onClick: function (e) {
        reactState(e, 'dig', 1); playVoice(this.voice);
        dropGroundItem('truffle', e.x); findEgg('boar-truffle');
      },
    },
    {
      key: 'mouse', name: 'Mouse', w: 3, t: 'A', layer: 2, speed: 60, kind: 'quad',
      shape: shape({ bw: 7, bh: 5, hw: 4, hh: 4, lh: 2, lw: 1, tw: 9, th: 1, tailShape: 'long', earShape: 'round', ew: 2, eh: 3, snout: 2 }),
      pal: P('#9C9084', '#6E6459'),
      voice: { freqs: [2000, 2200], gap: 40, d: 0.05 }, onClick: function (e) {
        reactState(e, 'hide', 0.6); playVoice(this.voice);
        const rock = W().nearestProp(e.x, 'rock'); exitToward(e, rock ? rock.x : e.x + (e.dir || 1) * 80); findEgg('mouse-hide');
      },
    },
    {
      key: 'badger', name: 'Badger', w: 2, t: 'N', layer: 2, speed: 14, kind: 'quad',
      shape: shape({ bw: 14, bh: 9, lh: 4, mask: true, snout: 4, earShape: 'round', tailShape: 'stub' }),
      pal: P('#B9B4AC', '#3A3A3A'),
      voice: { freqs: [200, 150] }, onClick: function (e) { reactState(e, 'dig', 0.8); playVoice(this.voice); e.data.willVanish = true; },
    },
    {
      key: 'cat', name: 'Cat', w: 0, t: 'A', layer: 2, speed: 20, kind: 'quad',
      shape: shape({ bw: 11, bh: 8, tw: 7, th: 9 }),
      pal: P('#4A4A4E', '#2C2C2E'),
      gate: function () { return (W().t - (W().lastInteractAt || 0)) > 60; },
      voice: { freqs: [600, 700, 600], gap: 110 }, onClick: function (e) {
        reactState(e, 'purr', 2); W().pet('cat'); playVoice(this.voice);
        e.data.follow = true;
      },
    },
    {
      key: 'dragonfly', name: 'Dragonfly', w: 6, t: 'D', layer: 'pond', speed: 26, kind: 'flutter', pal: P('#3EC7C0', '#63E0DA'),
      voice: { freqs: [1700, 2000], gap: 30, d: 0.05 }, onClick: function (e) { reactState(e, 'zip', 0.7); playVoice(this.voice); },
    },
    {
      key: 'bee', name: 'Bee', w: 4, t: 'D', layer: 2, speed: 30, kind: 'flutter', pal: P('#F4C542', '#2A2A2A'),
      voice: { freqs: [300], t: 'sawtooth', d: 0.4 }, onClick: function (e) {
        reactState(e, 'buzz', 0.6); playVoice(this.voice);
        const n = ((W().flags.beeFollows as number) || 0) + 1;
        W().flags.beeFollows = n;
        if (n >= 3) findEgg('bee-hive');
      },
    },
    {
      key: 'snail', name: 'Snail', w: 1, t: 'A', layer: 2, speed: 3, kind: 'quad',
      shape: shape({ bw: 10, bh: 7, ears: false, tail: false, legs: false, shell: true, neck: 2 }),
      pal: P('#8A9A6E', '#5E6C48', '#D9E6BE'),
      voice: { freqs: [400], t: 'sine', d: 0.2 }, onClick: function (e) {
        reactState(e, 'retract', 1); playVoice(this.voice);
        if (W().nearPond(e.x)) { W().dropCarry(); findEgg('snail-lily'); return; }
        // F13: away from the water, an empty-handed click lets you carry it to the pond
        if (!W().carry) { e.state = 'gone'; W().pickUp('snail', e.x, e.y); }
      },
    },
    {
      key: 'otter', name: 'Otter', w: 2, t: 'D', layer: 'pond', speed: 14, kind: 'quad',
      shape: shape({ bw: 14, bh: 8, tw: 10, th: 3, tailShape: 'long', earShape: 'round', ew: 2, eh: 2, snout: 3, lh: 3 }),
      pal: P('#5E4A38', '#3E3024'),
      voice: { freqs: [700, 850], gap: 80 }, onClick: function (e) {
        if (W().carry === 'fish') {
          W().dropCarry(); reactState(e, 'juggle', 1.4); playVoice({ freqs: [700, 900, 1100], gap: 70 }); findEgg('fish-otter'); return;
        }
        reactState(e, 'float', 1.2); playVoice(this.voice);
      },
    },
    {
      key: 'eagle', name: 'Eagle', w: 1, t: 'D', layer: 'sky', speed: 70, kind: 'bird', pal: P('#5A4A38', '#2E2419'),
      voice: { freqs: [1600, 1300], gap: 90 }, onClick: function (e) { reactState(e, 'dive', 1); playVoice(this.voice); },
    },
    {
      key: 'fish', name: 'Fish', w: 0, t: 'A', layer: 'pond', speed: 0, kind: 'fish',
      shape: shape({ bw: 12, bh: 6, ears: false, tail: true, legs: false, tw: 6, th: 5 }),
      pal: P('#6FA8DC', '#3D6FA8', '#DCE9F5'),
      voice: { freqs: [900, 1200], gap: 50 },
      onClick: function (e) { /* caught mid-air */
        if (e.data.caught) return;
        e.data.caught = true; e.state = 'gone';
        W().pickUp('fish', e.x, e.y);
        findEgg('pond-fish');
      },
    },
  ];

  SPECIES.length = 0;
  ALL.forEach((s) => { SPECIES_BY_KEY[s.key] = s; SPECIES.push(s); });
  return ALL;
}
