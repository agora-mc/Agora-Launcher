/**
 * The 43 fixed props — deterministic layout, same seed every load.
 *
 * Ported verbatim from v4-world.html. The props are built once with the
 * initial canvas width (as in the prototype); the pond and lily pads are
 * re-anchored to the live water span whenever the terrain regenerates.
 */

import type { GameApi } from './api';
import type { EngineState } from './state';
import { SPECIES_BY_KEY } from './species';
import type { Prop } from './types';

/** Deterministic x position from the world's seeded PRNG. */
export function buildProps(state: EngineState, api: GameApi): Prop[] {
  const { world: W, dropGroundItem, findEgg, shakeProp, blip } = api;
  const wrand = state.wrand;

  function seededX(lo: number, hi: number): number { return lo + wrand() * (hi - lo); }

  const arr: Prop[] = [];
  function add(o: Prop): Prop {
    o.hb = o.hb || { w: (o.w as number || 30), h: (o.h as number || 36) };
    arr.push(o);
    return o;
  }

  const oakXs = [seededX(0.10, 0.20) * state.W, seededX(0.34, 0.42) * state.W, seededX(0.80, 0.90) * state.W];
  oakXs.forEach((x, i) => {
    add({
      key: 'oak', name: 'Oak tree', layer: 2, x: x, w: 34, h: 64, hole: false, shakeT: 0,
      reaction: function (p: Prop) {
        shakeProp(p); if (Math.random() < 0.1) { W().spawn(SPECIES_BY_KEY.squirrel, { x: p.x, fromLeft: Math.random() < 0.5 }); }
        dropGroundItem('acorn', p.x);
      },
    });
    void i;
  });
  const pineXs = [seededX(0.55, 0.62) * state.W, seededX(0.68, 0.75) * state.W];
  pineXs.forEach((x) => {
    add({
      key: 'pine', name: 'Pine tree', layer: 1, x: x, w: 26, h: 70, shakeT: 0,
      reaction: function (p: Prop) {
        shakeProp(p); dropGroundItem(state.WEATHER[state.weather] === 'snow' ? 'snowball' : 'pinecone', p.x);
      },
    });
  });
  add({
    key: 'pond', name: 'Pond', layer: 2, x: state.W * 0.30, rw: 130, h: 26, ripple: 0,
    reaction: function (p: Prop) {
      p.ripple = 1; blip(300, 0.3, 'sine', 0.06);
      // F10: an empty-handed click lets you scoop up water
      if (!W().carry) W().pickUp('water', p.x, 0);
      // F3: the fish is caught mid-air (see fish.onClick) — discovery rewards catching
      W().spawnFish(p);
      const n = ((W().flags.pondClicks as number) || 0) + 1;
      W().flags.pondClicks = n;
    },
    hb: { w: 260, h: 40 },
  });
  [seededX(0.15, 0.22), seededX(0.44, 0.5), seededX(0.72, 0.78)].forEach((fx) => {
    add({
      key: 'flowers', name: 'Flower patch', layer: 2, x: fx * state.W, w: 26, h: 14, bloom: false, picked: false,
      reaction: function (p: Prop) {
        p.bloom = true; blip(700, 0.12); findEgg('flower-bloom');
        // F10: once bloomed, a flower can be picked and given to the deer
        if (p.bloom && !p.picked) { p.picked = true; dropGroundItem('flower', (p.x as number) + 14); }
        for (let i = 0; i < 2; i++) W().spawn(SPECIES_BY_KEY.bee, { x: (p.x as number) + (i - 1) * 30 });
        const wf = (W().flags.wateredFlowers = (W().flags.wateredFlowers as Record<string, boolean>) || {});
        if (W().carry === 'water') {
          wf[p.x as unknown as string] = true; W().dropCarry();
          if (Object.keys(wf).length >= 3) findEgg('water-flowers');
        }
      },
    });
  });
  [seededX(0.24, 0.29), seededX(0.5, 0.55), seededX(0.62, 0.66), seededX(0.85, 0.92)].forEach((rx) => {
    add({
      key: 'rock', name: 'Rock', layer: 2, x: rx * state.W, w: 20, h: 14, tipped: false,
      reaction: function (p: Prop) { p.tipped = true; blip(240, 0.15, 'square'); findEgg('rock-beetle'); },
    });
  });
  add({
    key: 'log', name: 'Fallen log', layer: 2, x: state.W * 0.62, w: 60, h: 12, shrooms: false,
    reaction: function (p: Prop) { p.shrooms = true; blip(500, 0.1); findEgg('log-shroom'); },
  });
  [seededX(0.18, 0.24), seededX(0.68, 0.73)].forEach((bx) => {
    add({
      key: 'bush', name: 'Berry bush', layer: 2, x: bx * state.W, w: 24, h: 16, berries: true,
      reaction: function (p: Prop) {
        if (p.berries) {
          dropGroundItem('berry', p.x);
          p.berries = false;
          setTimeout(function () { p.berries = true; }, 15000);
          blip(560, 0.1);
        }
      },
    });
  });
  const ringCX = state.W * 0.46;
  for (let ri = 0; ri < 7; ri++) {
    const rx = ringCX + Math.cos(ri / 7 * 6.283) * 22;
    add({
      key: 'mushroom', ringIdx: ri, name: 'Mushroom', layer: 2, x: rx, w: 8, h: 8,
      reaction: function () { blip(620 + ri * 20, 0.08); W().ringHit(ri); },
    });
  }
  add({
    key: 'hive', name: 'Beehive', layer: 2, x: (oakXs[1] as number) + 18, w: 16, h: 14, y0: 0, clicks: 0, honey: false,
    reaction: function (p: Prop) {
      p.clicks = (p.clicks as number) + 1; blip(340, 0.1, 'sawtooth');
      for (let i = 0; i < 2; i++) W().spawn(SPECIES_BY_KEY.bee, { x: p.x });
      if ((p.clicks as number) >= 3) { p.honey = true; dropGroundItem('honey', p.x); }
    },
  });
  add({
    key: 'campfire', name: 'Campfire', layer: 2, x: state.W * 0.56, w: 20, h: 16, clicks: 0, lit: false,
    reaction: function (p: Prop) {
      p.clicks = (p.clicks as number) + 1; blip(260, 0.14, 'square');
      if ((p.clicks as number) >= 3 && !p.lit) {
        p.lit = true; blip(180, 0.4, 'sawtooth', 0.1);
        if (W().isNight()) W().startCampfireTales(p);
      }
    },
  });
  add({ key: 'stump', name: 'Stump', layer: 2, x: state.W * 0.38, w: 20, h: 8, reaction: function () { blip(300, 0.12); } });
  add({
    key: 'cave', name: 'Cave mouth', layer: 1, x: state.W * 0.94, w: 44, h: 40, blink: 0, glow: false,
    reaction: function (p: Prop) {
      p.blink = 1; blip(200, 0.3, 'sine');
      if (W().carry === 'firefly') { p.glow = true; W().dropCarry(); findEgg('firefly-cave'); }
      if (W().flags.haveKey) { p.opened = true; findEgg('the-long-con'); }
    },
  });
  add({
    key: 'scarecrow', name: 'Scarecrow', layer: 2, x: state.W * 0.68, w: 16, h: 34, spin: 0, feather: false,
    reaction: function (p: Prop) {
      p.spin = 1; blip(360, 0.15);
      if (W().carry === 'feather') { p.feather = true; W().dropCarry(); findEgg('feather-scarecrow'); }
    },
  });
  add({
    key: 'lily', name: 'Lily pads', layer: 'pond', x: state.W * 0.30, w: 20, h: 6, hb: { w: 24, h: 10 },
    reaction: function () { if (W().carry === 'snail') { W().dropCarry(); findEgg('snail-lily'); } },
  });
  add({ key: 'cattails', name: 'Cattails', layer: 2, x: state.W * 0.24, w: 10, h: 20, reaction: function () { blip(900, 0.08); } });
  add({
    key: 'boulder', name: 'Boulder', layer: 2, x: state.W * 0.88, w: 30, h: 22, clicks: 0, rolled: false,
    reaction: function (p: Prop) {
      p.clicks = (p.clicks as number) + 1; blip(220, 0.18, 'square');
      if ((p.clicks as number) >= 5 && !p.rolled) {
        p.rolled = true; W().flags.boulderMoved = true; W().flags.boulderX = p.x; findEgg('boulder-hole');
      }
    },
  });
  const SIGNS = ['Nether: 3km', "Home: you're soaking in it", 'The End: keep walking', 'Village: ask a llama', "Ravine: don't"];
  add({
    key: 'signpost', name: 'Signpost', layer: 2, x: state.W * 0.5, w: 14, h: 30, idx: 0,
    reaction: function (p: Prop) { p.idx = ((p.idx as number) + 1) % SIGNS.length; blip(500, 0.08); },
  });
  add({ key: 'anthill', name: 'Anthill', layer: 2, x: state.W * 0.42, w: 14, h: 10, reaction: function () { blip(1200, 0.05); } });
  add({
    key: 'sunmoon', name: 'Sun / Moon', layer: 'sky', x: 0, y: 0, w: 34, h: 34, phase: 0,
    reaction: function (p: Prop) {
      if (Math.sin(state.tod * Math.PI) > 0.4) { state.flash = Math.max(state.flash, 0.6); blip(880, 0.2); }
      else { p.phase = ((p.phase as number) + 1) % 4; blip(500, 0.2); }
    },
  });
  add({
    key: 'clouds', name: 'Cloud', layer: 'sky', x: state.W * 0.62, y: state.H * 0.16, w: 60, h: 20, rainT: 0,
    reaction: function (p: Prop) { p.rainT = 5; blip(340, 0.2, 'sine', 0.05); },
  });
  // snowman chain: three piles, only interactive in snow weather
  const snowXs = [state.W * 0.20, state.W * 0.255, state.W * 0.30];
  ([['big', 22, 0], ['medium', 15, 1], ['small', 9, 2]] as Array<[string, number, number]>).forEach((s, idx) => {
    add({
      key: 'snowpile', name: 'Snow pile', layer: 2, x: snowXs[idx], w: s[1], h: s[1] * 0.7, rank: s[2],
      visibleIf: function () { return state.WEATHER[state.weather] === 'snow'; },
      reaction: function (p: Prop) {
        const need = (W().flags.snowmanStep as number) || 0;
        if ((p.rank as number) === need) {
          W().flags.snowmanStep = need + 1; blip(500 + need * 100, 0.15);
          if ((W().flags.snowmanStep as number) >= 3 && !W().flags.snowmanBuilt) {
            W().flags.snowmanBuilt = true;
            W().props.push({
              key: 'snowman', name: 'Snowman', layer: 2, x: snowXs[1], w: 16, h: 26, waved: false, hb: { w: 24, h: 34 },
              visibleIf: function () { return state.WEATHER[state.weather] === 'snow'; },
              reaction: function (sp2: Prop) { sp2.waved = true; blip(660, 0.2); findEgg('snowman'); },
            });
          }
        } else { W().flags.snowmanStep = 0; blip(180, 0.2, 'square'); }
      },
    });
  });
  // constellation chain: five fixed stars, only interactive at night
  const starXs = [0.15, 0.30, 0.45, 0.60, 0.75].map((f) => f * state.W);
  const starRanks = [3, 5, 1, 4, 2];
  starXs.forEach((sx, i) => {
    add({
      key: 'star', name: 'Star', layer: 'sky', x: sx, y: state.H * (0.08 + 0.05 * i), w: 8, h: 8, rank: starRanks[i],
      visibleIf: function () { return W().isNight(); },
      reaction: function (p: Prop) {
        const order = [5, 4, 3, 2, 1];
        const need = order[(W().flags.constStep as number) || 0];
        if (p.rank === need) {
          W().flags.constStep = ((W().flags.constStep as number) || 0) + 1; blip(700 + (p.rank as number) * 80, 0.12);
          if ((W().flags.constStep as number) >= 5) { findEgg('constellation'); W().popFx(state.W * 0.5, state.H * 0.15, '* * * * *'); }
        } else { W().flags.constStep = 0; blip(160, 0.2, 'square'); }
      },
    });
  });

  return arr;
}

export function shakeProp(p: Prop): void {
  (p as Prop & { shakeT: number }).shakeT = 0.5;
}
