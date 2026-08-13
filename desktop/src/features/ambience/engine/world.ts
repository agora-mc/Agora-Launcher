/**
 * The living world engine core — ported 1:1 from v4-world.html.
 *
 * WORLD owns entities, props, items, carrying, the spawner, the update/draw
 * state machine, and the interaction dispatch. Everything stores WORLD x
 * coordinates and transforms to screen only at draw/hit time (F7), so
 * entities, props and terrain share one transform and can never drift apart
 * under mouse parallax.
 *
 * The prototype reached its helper functions (findEgg, dropGroundItem,
 * playVoice, burst, blip) as globals; here they arrive bound through `opts`
 * and the GameApi so the method bodies stay the same.
 */

import type { GameApi } from './api';
import { chord, blip as sfxBlip } from './audio/sfx';
import { makeFindEgg, checkAchievements } from './eggs';
import { ITEMS } from './items';
import { buildProps, shakeProp } from './props';
import type { EngineState, AmbienceEvent } from './state';
import { paraX, paraY, groundYWorld, screenOf, waterSurfaceY, waterSpan } from './terrain';
import { SPECIES, SPECIES_BY_KEY, buildSpecies, drawBird, drawFlutter, drawQuad, P } from './species';
import type { Entity, Prop, Species, WorldState, WorldItem } from './types';

/** World helpers the species/props handlers close over (prototype globals). */
export function reactState(e: Entity, fx: string, dur?: number): void {
  e.state = 'react'; e.fx = fx; e.t = 0; e.reactDur = dur || 1.2;
}

export function fleeState(e: Entity, mul?: number): void {
  e.state = 'flee';
  e.dir = e.x < 0 ? 1 : (e.x > 1 ? 1 : e.dir);
  e.vx = Math.abs(e.vx || 24) * (mul || 2.4) * (e.x > 0.5 ? -1 : 1) * (e.dir || 1) / Math.abs(e.dir || 1);
}

export function boltState(e: Entity): void {
  e.state = 'flee'; e.vx = (e.dir || (Math.random() < 0.5 ? 1 : -1)) * 140;
}

export function exitToward(e: Entity, tx: number): void {
  e.state = 'exit';
  e.dir = tx > e.x ? 1 : -1;
  e.vx = Math.abs(e.vx || 24) * e.dir * 1.6;
  e.data.exitTarget = tx;
}

export function dropGroundItem(state: EngineState, itemId: string, x: number): void {
  if (!ITEMS[itemId]) return;
  const world = state.world as WorldState;
  // F7: items store a WORLD x; the draw/hit passes derive screen y via screenOf
  world.items.push({ id: itemId, x: x, y: 0, t: 0 });
  if (world.items.length > 16) world.items.shift();
}

export function playVoice(state: EngineState, v: { seq?: number[]; freqs?: number[]; d?: number; t?: OscillatorType; v?: number; gap?: number } | null | undefined): void {
  if (!v) return;
  if (v.seq) {
    v.seq.forEach((f, i) => {
      setTimeout(() => sfxBlip(state, f, v.d || 0.14, v.t || 'triangle', v.v || 0.08), i * (v.gap || 90));
    });
  } else chord(state, v.freqs || [440], v.gap || 70);
}

export interface WorldOpts {
  blip: (f: number, d?: number, t?: OscillatorType, v?: number) => void;
  burst: (x: number, y: number, color: string, n?: number, spread?: number) => void;
  emit: (ev: AmbienceEvent) => void;
}

let uidc = 1;

/**
 * Build the whole living world: the WORLD object, the species catalogue, the
 * fixed props. Called once by the engine when the canvas is sized.
 */
export function createWorld(state: EngineState, opts: WorldOpts): WorldState {
  const { burst } = opts;

  const world: WorldState = {
    t: 0, entities: [], props: [], items: [], carry: null, flags: {}, found: {}, hover: null, hoverIsProp: false,
    dl: 0.5, spawnTimer: 3, sinceLastSpawn: 0, worldFx: [],
    layerScale: function (layer) { return layer === 0 ? 0.55 : layer === 1 ? 0.75 : layer === 2 ? 1 : layer === 'sky' ? 0.85 : 1; },
    isNight: function () { return this.dl < 0.35; },
    isDay: function () { return this.dl >= 0.35; },
    eligible: function (sp) {
      if (sp.t === 'D' && !this.isDay()) return false;
      if (sp.t === 'N' && !this.isNight()) return false;
      if (sp.gate && !sp.gate()) return false;
      return true;
    },
    spawn: function (sp, opts2) {
      opts2 = opts2 || {};
      const fromLeft = opts2.fromLeft !== undefined ? opts2.fromLeft : Math.random() < 0.5;
      // F6: species may declare a layerRange so the same animal appears at
      // varying depths (and sizes) across spawns
      const layer = sp.layerRange
        ? sp.layerRange[0] + Math.floor(Math.random() * (sp.layerRange[1] - sp.layerRange[0] + 1))
        : sp.layer;
      const scale = this.layerScale(layer) * (sp.size || 1);
      const e: Entity = {
        sp: sp, layer: layer, dir: fromLeft ? 1 : -1, vx: (sp.speed || 18) * (fromLeft ? 1 : -1),
        scale: scale, state: 'walk', t: 0, stateT: 0, phase: Math.random() * 6.28, fx: null, data: opts2.data || {},
        hb: { w: 26 * scale, h: 22 * scale }, removeAt: null, x: 0, y: 0,
      };
      if (layer === 'sky') {
        e.x = fromLeft ? -60 : state.W + 60; e.y = state.H * (0.08 + Math.random() * 0.28);
        e.vx = (sp.speed || 24) * (fromLeft ? 1 : -1);
      } else if (layer === 'pond') {
        const pond = this.pondProp();
        const pr = pond ? (pond.rw as number) * 0.7 : 120;
        const px0 = pond ? pond.x : state.W * 0.30;
        const span = waterSpan(state);
        // F9: pond creatures spawn inside the actual water, not a fixed ellipse
        e.x = span ? (span.x0 + Math.random() * (span.x1 - span.x0)) : (px0 + (Math.random() * 2 - 1) * pr);
        e.pondCX = px0; e.pondR = pr;
        e.vx = (sp.speed || 8) * (fromLeft ? 1 : -1); e.y = 0;
      } else {
        e.x = fromLeft ? -60 : state.W + 60;
        e.y = 0;
      }
      if (opts2.x !== undefined) e.x = opts2.x;
      // F8a: pond & perch residents get a lifespan so they don't crowd out the world
      if (layer === 'pond' || sp.perch) { e.residentUntil = this.t + 25 + Math.random() * 45; }
      this.entities.push(e);
      return e;
    },
    pondProp: function () {
      if (!this._pond) this._pond = this.props.filter(function (p) { return p.key === 'pond'; })[0];
      return this._pond;
    },
    despawn: function (e) { const i = this.entities.indexOf(e); if (i >= 0) this.entities.splice(i, 1); },
    hit: function (px, py) {
      for (let k = this.items.length - 1; k >= 0; k--) {
        const it = this.items[k];
        const s = screenOf(state, it.x, 2);
        if (px > s.x - 12 && px < s.x + 12 && py > s.y - 18 && py < s.y + 8) return { kind: 'item' as const, obj: it };
      }
      for (let i = this.entities.length - 1; i >= 0; i--) {
        const e = this.entities[i]; if (e.state === 'gone') continue;
        const m = 6;
        const sx = e.layer === 'sky' ? e.x + paraX(state, 'sky') : e.x + paraX(state, e.layer);
        const gy = e.layer === 'sky' ? e.y
          : e.layer === 'pond' ? (e.state === 'jump' ? (e.jy || waterSurfaceY(state)) : waterSurfaceY(state))
            : groundYWorld(state, e.x, e.layer) + paraY(state, e.layer) - e.hb.h * 0.5;
        const hw = e.hb.w / 2 + m, hh = e.hb.h / 2 + m;
        if (px > sx - hw && px < sx + hw && py > gy - hh && py < gy + hh) return { kind: 'entity' as const, obj: e };
      }
      for (let j = this.props.length - 1; j >= 0; j--) {
        const p = this.props[j]; if (p.visibleIf && !p.visibleIf()) continue;
        const hw2 = (p.hb ? p.hb.w : 30) / 2 + 6;
        const sx = p.layer === 'sky' ? p.x + paraX(state, 'sky') : p.x + paraX(state, p.layer);
        const pgy = p.key === 'pond' ? waterSurfaceY(state)
          : p.layer === 'sky' ? (p.y !== undefined ? p.y : state.H * 0.16)
            : p.layer === 'pond' ? waterSurfaceY(state)
              : (p.y !== undefined ? p.y + paraY(state, p.layer) : groundYWorld(state, p.x, p.layer) + paraY(state, p.layer));
        if (px > sx - hw2 && px < sx + hw2 && py > pgy - (p.hb ? p.hb.h : 30) - 6 && py < pgy + 6) return { kind: 'prop' as const, obj: p };
      }
      return null;
    },
    nearestProp: function (x, key) {
      let best: Prop | null = null, bd = 1e9;
      this.props.forEach(function (p) {
        if (p.key !== key) return;
        const d = Math.abs(p.x - x); if (d < bd) { bd = d; best = p; }
      });
      return best;
    },
    nearPond: function (x) { const pd = this.pondProp(); return !!pd && Math.abs(x - pd.x) < (pd.rw as number); },
    pet: function (species) {
      const petted = (this.flags.petted = (this.flags.petted as Record<string, boolean>) || {});
      petted[species] = true;
      if (Object.keys(petted).length >= 5) this.emit?.({ type: 'achievement', icon: '🤲', name: 'Gentle Hand' });
    },
    dropCarry: function () {
      this.carry = null;
      this.emit?.({ type: 'drop-carry' });
    },
    pickUp: function (itemId, x, y) {
      if (this.carry) { this.items.push({ id: this.carry, x: x, y: y, t: 0 }); }
      this.carry = itemId;
      const carried = (this.flags.carriedTypes = (this.flags.carriedTypes as Record<string, boolean>) || {});
      carried[itemId] = true;
      if (Object.keys(carried).length >= 14) this.emit?.({ type: 'achievement', icon: '🎒', name: 'Collector' });
      // carry tag (prototype's #carryTag) is the React shell's job via the event
      const seen = (this.flags.seenTag = (this.flags.seenTag as Record<string, boolean>) || {});
      const firstTime = !seen[itemId];
      seen[itemId] = true;
      this.lastPickup = { itemId: itemId, name: ITEMS[itemId] ? ITEMS[itemId].name : itemId, firstTime: firstTime, at: this.t };
      this.emit?.({ type: 'carry', itemId: itemId, name: ITEMS[itemId] ? ITEMS[itemId].name : itemId, firstTime: firstTime });
      opts.blip(760, 0.09);
    },
    spawnFish: function (pond) {
      const waterY = waterSurfaceY(state);
      const f: Entity = {
        sp: SPECIES_BY_KEY.fish, layer: 'pond', x: pond.x + (Math.random() * 80 - 40),
        y: 0, jy: waterY, vy: -330, waterY: waterY, vx: (Math.random() < 0.5 ? -1 : 1) * 26,
        dir: 1, scale: 1.4, state: 'jump', t: 0, stateT: 0, fx: null, hb: { w: 26, h: 22 }, data: {}, phase: 0, removeAt: null,
      };
      this.entities.push(f); this.flags.fishJumping = f;
      this.splash(pond.x, waterY);
      if ((this.flags.pondClicks as number) >= 2 && this.flags.truffleThrown) this._findEgg?.('truffle-pond');
      return f;
    },
    splash: function (x, y) {
      opts.blip(300, 0.18, 'sine', 0.05);
      burst(x, y, '#8FD3FF', 12, 6);
    },
    spawnRainbow: function () {
      if (this.flags.rainbowUp) return; this.flags.rainbowUp = true; this.flags.rainbowEnds = {};
      const lx = state.W * 0.22, rx = state.W * 0.78;
      this.props.push({
        key: 'rainbow-end', side: 'l', name: 'Rainbow (left end)', layer: 'sky', x: lx, y: state.H * 0.55, pairX: rx, w: 16, h: 16,
        reaction: function () {
          (world.flags.rainbowEnds as Record<string, boolean>).l = true; opts.blip(700, 0.15); world.checkRainbow();
        },
      });
      this.props.push({
        key: 'rainbow-end', side: 'r', name: 'Rainbow (right end)', layer: 'sky', x: rx, y: state.H * 0.55, pairX: lx, w: 16, h: 16,
        reaction: function () {
          (world.flags.rainbowEnds as Record<string, boolean>).r = true; opts.blip(760, 0.15); world.checkRainbow();
        },
      });
    },
    checkRainbow: function () {
      const ends = this.flags.rainbowEnds as Record<string, boolean> | undefined;
      if (ends && ends.l && ends.r && !this.found['rainbow']) {
        this._findEgg?.('rainbow');
        dropGroundItem(state, 'coin', state.W * 0.5);
        this.props = this.props.filter(function (p) { return p.key !== 'rainbow-end'; });
        this.flags.rainbowUp = false;
      }
    },
    spawnSwarm: function () {
      const s: Entity = {
        sp: SWARM_SP, layer: 2, x: state.W * 0.5, y: 0, vx: 0, dir: 1, scale: 2,
        state: 'idle', fx: null, t: 0, stateT: 9999, hb: { w: 60, h: 30 }, data: { isSwarm: true }, phase: 0, removeAt: null,
      };
      this.entities.push(s);
    },
    campfireClick: function (e, key) {
      reactState(e, 'happy', 1);
      playVoice(state, { freqs: [600, 760, 900], gap: 80 });
      const got = (this.flags.campfireClicked = (this.flags.campfireClicked as Record<string, boolean>) || {});
      got[key] = true;
      if (got.fox && got.rabbit && got.deer) this._findEgg?.('campfire-tales');
    },
    startCampfireTales: function (fire) {
      const order = ['fox', 'rabbit', 'deer'];
      this.flags.campfireGathered = [];
      order.forEach((key, i) => {
        setTimeout(() => {
          const e = this.spawn(SPECIES_BY_KEY[key], { x: fire.x + (i - 1) * 40, fromLeft: Math.random() < 0.5 });
          e.state = 'idle'; e.data.atCampfire = true; e.vx = 0;
        }, i * 1600);
      });
    },
    ringHit: function (idx) {
      const hits = (this.flags.ringHits = (this.flags.ringHits as Record<string, number>) || {});
      hits[idx] = this.t;
      const keys = Object.keys(hits), self = this;
      const recent = keys.filter(function (k) { return self.t - (hits[k] as number) < 6; });
      if (recent.length >= 7) this._findEgg?.('fairy-ring');
    },
    update: function (dt) {
      this.t += dt;
      if (state.reduce) { return; } // props stay static, no wanderers, no ambient motion
      const TIMERFIELDS = ['shakeT', 'ripple', 'blink', 'spin', 'rainT'];
      this.props.forEach(function (p) {
        TIMERFIELDS.forEach(function (f) { if ((p[f] as number) > 0) p[f] = Math.max(0, (p[f] as number) - dt); });
      });
      // F8c: watchdog — the world must never go quiet. If nothing has spawned in
      // 20s, evict the longest-staying resident and force a spawn.
      this.sinceLastSpawn = (this.sinceLastSpawn || 0) + dt;
      if (this.sinceLastSpawn > 20) {
        const oldest = this.entities.filter(function (e) { return e.residentUntil; })
          .sort(function (a, b) { return (a.residentUntil as number) - (b.residentUntil as number); })[0];
        if (oldest) oldest.residentUntil = 0;
        this.spawnTimer = 0;
      }
      this.spawnTimer -= dt;
      // F8b: cap only ambient wanderers — residents (pond/perch) don't starve the
      // spawner, so the world keeps breathing instead of filling to a hard cap.
      const ambient = this.entities.filter(function (e) { return !e.residentUntil && e.state !== 'sleep'; }).length;
      if (this.spawnTimer <= 0 && ambient < 9 && this.entities.length < 16) {
        this.spawnTimer = 4 + Math.random() * 5;
        this.sinceLastSpawn = 0;
        // F11: gated species (w:0 + gate) spawn directly — the cat can finally appear
        const special = SPECIES.filter(function (sp2) { return sp2.w === 0 && sp2.gate && sp2.gate(); });
        if (special.length && !this.entities.some(function (e2) { return e2.sp === special[0]; })) {
          this.spawn(special[0], {});
          return;
        }
        const pool = SPECIES.filter(this.eligible.bind(this));
        const totalW = pool.reduce(function (s, sp) { return s + sp.w; }, 0);
        if (totalW > 0) {
          let r = Math.random() * totalW, sp = pool[0];
          for (let i = 0; i < pool.length; i++) { r -= pool[i].w; if (r <= 0) { sp = pool[i]; break; } }
          if (sp.perch) {
            /* owls/crows/woodpeckers perch on a fixed spot near a tree/scarecrow */
            const host = this.nearestProp(state.W * Math.random(), sp.key === 'woodpecker' ? 'oak' : (sp.key === 'crow' ? 'scarecrow' : 'pine'));
            this.spawn(sp, { x: host ? host.x + 14 : state.W * (0.2 + Math.random() * 0.6) });
          } else if (sp.group) {
            const lead = this.spawn(sp, {});
            const members: Entity[] = [lead];
            for (let g = 0; g < 4 + Math.floor(Math.random() * 3); g++) {
              const m = this.spawn(sp, { x: lead.x - (g + 1) * 26 * lead.dir, fromLeft: lead.dir > 0 });
              m.groupOffset = (g + 1);
              members.push(m);
            }
            members.forEach(function (m) { m.groupMembers = members; });
          } else if (sp.layer === 'pond') {
            this.spawn(sp, {});
            if (sp.key === 'duck') {
              const lead2 = this.entities[this.entities.length - 1];
              lead2.data.ducklings = [];
              for (let d = 0; d < 3; d++) (lead2.data.ducklings as Array<{ ox: number; oy: number }>).push({ ox: -(d + 1) * 10, oy: 0 });
            }
          } else { this.spawn(sp, {}); }
        }
      }
      for (let idx = this.entities.length - 1; idx >= 0; idx--) {
        const e = this.entities[idx]; e.t += dt; e.uid = e.uid || ('e' + (uidc++)); e.data = e.data || {};
        const sp = e.sp;
        if (e.state === 'react') {
          if (e.t >= (e.reactDur || 1.2)) {
            if (e.fx === 'curl') { e.state = 'sleep'; e.sleepStart = this.t; e.sleepFor = 30 + Math.random() * 30; }
            else if (e.fx === 'shell') { e.state = 'walk'; e.fx = null; }
            else { e.state = 'walk'; e.fx = null; }
          }
        } else if (e.state === 'sleep') {
          // F8a: sleeping animals wake and wander off — nothing is terminal
          if (this.t - (e.sleepStart || 0) > (e.sleepFor || 30)) { e.state = 'walk'; e.fx = null; e.sleepFor = 0; }
        } else if (e.state === 'walk') {
          e.phase += dt * 6;
          if (e.layer === 'sky') { e.x += e.vx * dt; e.y += Math.sin(e.t * 1.4 + e.phase) * 6 * dt * 10 * 0.02; }
          else if (e.layer === 'pond') {
            e.x += e.vx * dt * 0.4;
            const ws = waterSpan(state);
            if (ws) { if (e.x > ws.x1 - 10) e.vx = -Math.abs(e.vx || 8); if (e.x < ws.x0 + 10) e.vx = Math.abs(e.vx || 8); }
            else if (e.pondCX !== undefined) { if (e.x > e.pondCX + (e.pondR || 0) || e.x < e.pondCX - (e.pondR || 0)) e.vx *= -1; }
          }
          else if (!sp.perch) {
            e.x += e.vx * dt;
            if (sp.kind === 'quad' && Math.random() < dt * 0.15) { e.state = 'idle'; e.stateT = 0.6 + Math.random() * 1.4; }
          }
          if (sp.behave) sp.behave(e, dt);
        } else if (e.state === 'idle') {
          e.stateT -= dt; if (e.stateT <= 0) e.state = 'walk';
        } else if (e.state === 'flee') {
          e.x += e.vx * dt; e.phase += dt * 12;
        } else if (e.state === 'jump') { // F3: fish arc — gravity, then splash back down
          e.vy = (e.vy || 0) + 900 * dt; e.jy = (e.jy || 0) + (e.vy || 0) * dt; e.x += e.vx * dt;
          if ((e.jy || 0) >= (e.waterY || 0) && (e.vy || 0) > 0) {
            this.splash(e.x, e.waterY || 0);
            e.state = 'gone';
            if (this.flags.fishJumping === e) this.flags.fishJumping = null;
          }
        } else if (e.state === 'exit') {
          const tx = e.data.exitTarget as number;
          const dir = tx > e.x ? 1 : -1;
          e.x += Math.abs(e.vx) * dir * dt;
          e.phase += dt * 8;
          if (Math.abs(e.x - tx) < 6) {
            if (e.data.fetchQuest) {
              e.data.fetched = true; e.state = 'react'; e.fx = 'fetch'; e.t = 0; e.reactDur = 1;
              setTimeout(() => {
                this.flags.haveKey = true;
                // F5: sample the ground at the key's own world x, not x=0
                const cave = this.props.filter(function (p) { return p.key === 'cave'; })[0];
                dropGroundItem(state, 'key', cave ? cave.x - 30 : 0);
              }, 900);
            } else { e.state = 'gone'; }
          }
        }
        // F8a: residents leave on their own — nothing should live forever
        if (e.residentUntil && this.t > e.residentUntil && e.state !== 'react') {
          e.residentUntil = 0;
          e.state = 'exit';
          e.data.exitTarget = (e.x < state.W / 2) ? -160 : state.W + 160;
          e.vx = Math.abs(e.vx || 20) * (e.x < state.W / 2 ? -1 : 1) * 1.4;
        }
        // out of bounds despawn — test the parallax-transformed (screen) x (F7)
        const sxe = e.x + (e.layer === 'sky' ? paraX(state, 'sky') : paraX(state, e.layer));
        if (e.layer !== 'pond' && !sp.perch && e.state !== 'sleep' && (sxe < -140 || sxe > state.W + 140)) e.state = 'gone';
        if (e.data.willVanish && e.t > 1.6) e.state = 'gone';
        if (e.state === 'gone') this.entities.splice(idx, 1);
      }
      // ground items: gentle bob + despawn after 60s
      for (let ii = this.items.length - 1; ii >= 0; ii--) {
        const it = this.items[ii];
        it.t += dt;
        if (it.t > 60) this.items.splice(ii, 1);
      }
      // world fx (Zzz, hearts, notes)
      for (let fi = this.worldFx.length - 1; fi >= 0; fi--) {
        const wf = this.worldFx[fi];
        wf.life -= dt * 0.7;
        wf.y += wf.vy * dt;
        if (wf.life <= 0) this.worldFx.splice(fi, 1);
      }
    },
    popFx: function (x, y, text) {
      this.worldFx.push({ x: x, y: y, text: text, life: 1.4, vy: -16 });
      if (this.worldFx.length > 40) this.worldFx.shift();
    },
    drawEntity: function (ctx, e) {
      const sp = e.sp; if (!sp) return;
      // F7: draw at the parallax-transformed position; e.x stays a world coordinate
      const sx = e.layer === 'sky' ? e.x + paraX(state, 'sky') : e.x + paraX(state, e.layer);
      const gy = e.layer === 'sky' ? e.y
        : e.layer === 'pond' ? (e.state === 'jump' ? (e.jy || waterSurfaceY(state)) : waterSurfaceY(state))
          : groundYWorld(state, e.x, e.layer) + paraY(state, e.layer);
      const dir = (e.dir || 1) >= 0 ? 1 : -1;
      const pal = sp.pal || { body: '#888', dark: '#555', eye: '#161616' };
      let squash = 1, rot = 0, extraY = 0;
      const walking = e.state === 'walk' || e.state === 'flee' || e.state === 'exit';
      if (e.state === 'react' || e.state === 'sleep') {
        switch (e.fx) {
          case 'curl': case 'ball': case 'shell': squash = 1 - Math.min(0.5, e.t * 0.6); break;
          case 'roar': case 'happy': case 'friend': case 'pet': squash = 1 + Math.sin(e.t * 10) * 0.06; break;
          case 'rotate': rot = Math.min(1, e.t / 1.2) * Math.PI * 2; break;
          case 'loop': rot = e.t * 8; break;
          case 'thump': case 'peck': extraY = -Math.abs(Math.sin(e.t * 16)) * 3; break;
          default: break;
        }
      }
      ctx.save();
      if (rot) { ctx.translate(sx, gy); ctx.rotate(rot); ctx.translate(-sx, -gy); }
      let size: { w: number; h: number } | undefined;
      if (sp.kind === 'bird') {
        const by = gy - (sp.perch ? 16 * e.scale : 0) + extraY;
        size = drawBird(ctx, sx, by, e.scale, dir, pal, e.phase, walking || !!e.data.follow);
      } else if (sp.kind === 'flutter') {
        size = drawFlutter(ctx, sx, gy - 14 * e.scale, e.scale, e.phase + e.t, pal);
      } else if (sp.kind === 'fish') { // F3: arcing fish, nose follows the arc
        ctx.translate(sx, e.jy || gy);
        ctx.rotate((e.vy || 0) * 0.012);
        ctx.fillStyle = pal.body; ctx.fillRect(-6 * e.scale, -3 * e.scale, 12 * e.scale, 6 * e.scale);
        ctx.fillStyle = pal.accent || pal.body; ctx.fillRect(-10 * e.scale, -3 * e.scale, 4 * e.scale, 6 * e.scale);
        ctx.fillStyle = pal.belly || pal.body; ctx.fillRect(-6 * e.scale, 0, 12 * e.scale, 2 * e.scale);
        ctx.fillStyle = '#161616'; ctx.fillRect(3 * e.scale, -1 * e.scale, 1 * e.scale, 1 * e.scale);
        size = { w: 20, h: 14 };
      } else if (sp.kind === 'glow') {
        const a = e.fx === 'glow' ? 1 : (0.4 + 0.5 * Math.abs(Math.sin(world.t * 3 + e.x * 0.05)));
        ctx.globalAlpha = a;
        ctx.fillStyle = pal.body;
        ctx.shadowBlur = 10; ctx.shadowColor = pal.body;
        const gs = e.data.isSwarm ? 18 : 3;
        ctx.fillRect(Math.round(sx - gs / 2), Math.round(gy - gs / 2), gs, gs);
        ctx.shadowBlur = 0; ctx.globalAlpha = 1;
        size = { w: 10, h: 10 };
      } else {
        size = drawQuad(ctx, sx, gy + extraY, e.scale, dir, pal, e.phase, walking, sp.shape, squash);
      }
      ctx.restore();
      if (size) { e.hb.w = Math.max(18, size.w); e.hb.h = Math.max(16, size.h); }
      if (e.data && e.data.ducklings && sp.key === 'duck') {
        const ducklings = e.data.ducklings as Array<{ ox: number; oy: number }>;
        ducklings.forEach(function (d, i) {
          const tx = e.data.line ? sx - (i + 1) * 16 * dir : sx + d.ox;
          drawBird(ctx, tx, gy, e.scale * 0.5, dir, pal, e.phase + i * 0.6, true);
        });
      }
    },
    drawProp: function (ctx, p) {
      // F7: draw at the parallax-transformed position; p.x stays a world coordinate
      const sx = p.layer === 'sky' ? p.x + paraX(state, 'sky') : p.x + paraX(state, p.layer);
      const gy = p.layer === 'sky' ? (p.y !== undefined ? p.y : state.H * 0.16)
        : p.layer === 'pond' ? waterSurfaceY(state)
          : (p.y !== undefined ? p.y + paraY(state, p.layer) : groundYWorld(state, p.x, p.layer) + paraY(state, p.layer));
      const s = p.layer === 0 ? 0.55 : p.layer === 1 ? 0.75 : 1;
      const shk = (!state.reduce && (p.shakeT as number)) ? Math.sin((p.shakeT as number) * 40) * 3 * (p.shakeT as number) : 0;
      ctx.save();
      switch (p.key) {
        case 'oak': case 'pine': {
          const trunkW = Math.round((p.key === 'oak' ? 7 : 5) * s), trunkH = Math.round((p.h as number) * 0.42 * s);
          ctx.fillStyle = '#5A3C24';
          ctx.fillRect(Math.round(sx - trunkW / 2 + shk), Math.round(gy - trunkH), trunkW, trunkH);
          const cw = Math.round((p.w as number) * s), ch = Math.round((p.h as number) * 0.62 * s);
          ctx.fillStyle = p.key === 'oak' ? '#3B7A34' : '#2C5E3E';
          ctx.fillRect(Math.round(sx - cw / 2 + shk), Math.round(gy - trunkH - ch), cw, ch);
          if (p.hole) { ctx.fillStyle = '#1A1410'; ctx.fillRect(Math.round(sx - 3 + shk), Math.round(gy - trunkH * 0.6), 6, 8); }
          break;
        }
        case 'pond': { // F9: water fills the carved valley — blocky columns, no ellipse
          const stepW = 12, lvl = state.WATER_LEVEL + paraY(state, 2), offX = paraX(state, 2) - state.OVER;
          const basin = state.BASIN;
          if (basin && state.R3) {
            for (let i = basin.c - basin.half; i <= basin.c + basin.half; i++) {
              if (i < 0 || i >= state.R3.length) continue;
              const ty = state.R3[i] + paraY(state, 2);
              if (ty <= lvl) continue;                              // land, not water
              const x = i * stepW + offX;
              ctx.fillStyle = 'rgba(58,124,176,.78)';
              ctx.fillRect(Math.round(x), Math.round(lvl), stepW + 1, Math.round(ty - lvl));
              ctx.fillStyle = 'rgba(150,215,245,' + (0.5 + 0.3 * Math.sin(world.t * 2 + i * 0.6)) + ')';
              ctx.fillRect(Math.round(x), Math.round(lvl), stepW + 1, 3);
            }
          }
          // ripples: expanding horizontal bars along the surface
          if (!state.reduce && (p.ripple as number) > 0) {
            ctx.fillStyle = 'rgba(255,255,255,' + ((p.ripple as number) * 0.45) + ')';
            for (let r = 0; r < 3; r++) {
              const yy = lvl + 3 + r * 3, half = 16 + r * 10 + (1 - (p.ripple as number)) * 46;
              ctx.fillRect(Math.round(sx - half), Math.round(yy), Math.round(half * 2), 2);
            }
          }
          // lily pads float on the surface
          const span = waterSpan(state);
          for (let l = 0; l < 4; l++) {
            const lx = span.x0 + (span.x1 - span.x0) * ((l + 1) / 5) + paraX(state, 2);
            ctx.fillStyle = '#3D7031'; ctx.fillRect(Math.round(lx), Math.round(lvl - 3), 14, 5);
            ctx.fillStyle = '#2E5426'; ctx.fillRect(Math.round(lx + 2), Math.round(lvl - 2), 10, 3);
          }
          break;
        }
        case 'flowers': {
          for (let i = 0; i < 5; i++) {
            const fx2 = sx + (i - 2) * 7;
            ctx.fillStyle = p.bloom ? ['#E6659E', '#F4C542', '#8FD1E8', '#E9EDF0', '#C58BFF'][i] : '#3B7A34';
            ctx.fillRect(Math.round(fx2), Math.round(gy - (p.bloom ? 12 : 6)), 4, p.bloom ? 4 : 3);
          }
          break;
        }
        case 'rock': {
          ctx.fillStyle = '#7B7B82';
          const rw = p.w as number, rh = p.tipped ? (p.h as number) * 0.5 : (p.h as number);
          ctx.save();
          if (p.tipped) { ctx.translate(sx, gy - rh / 2); ctx.rotate(0.6); ctx.translate(-sx, -(gy - rh / 2)); }
          ctx.fillRect(Math.round(sx - rw / 2), Math.round(gy - rh), rw, rh);
          ctx.restore();
          break;
        }
        case 'log': {
          ctx.fillStyle = '#6B4A2E';
          ctx.fillRect(Math.round(sx - (p.w as number) / 2), Math.round(gy - (p.h as number)), p.w as number, p.h as number);
          if (p.shrooms) {
            ctx.fillStyle = '#D0392B';
            for (let m = 0; m < 4; m++) ctx.fillRect(Math.round(sx - (p.w as number) / 2 + m * (p.w as number) / 4), Math.round(gy - (p.h as number) - 4), 4, 4);
          }
          break;
        }
        case 'bush': {
          ctx.fillStyle = '#2E5E2A';
          ctx.fillRect(Math.round(sx - (p.w as number) / 2), Math.round(gy - (p.h as number)), p.w as number, p.h as number);
          if (p.berries) {
            ctx.fillStyle = '#4A2ED8';
            for (let b = 0; b < 3; b++) ctx.fillRect(Math.round(sx - (p.w as number) / 2 + 4 + b * 6), Math.round(gy - (p.h as number) + 3), 3, 3);
          }
          break;
        }
        case 'mushroom': {
          ctx.fillStyle = '#E9EDF0'; ctx.fillRect(Math.round(sx - 1), Math.round(gy - 5), 2, 5);
          ctx.fillStyle = '#D0392B'; ctx.fillRect(Math.round(sx - 4), Math.round(gy - 8), 8, 4);
          break;
        }
        case 'hive': {
          ctx.fillStyle = '#D9A441'; ctx.fillRect(Math.round(sx - 8), Math.round(gy - 40), 16, 14);
          if (p.honey) { ctx.fillStyle = '#B8860B'; ctx.fillRect(Math.round(sx - 2), Math.round(gy - 26), 4, 6); }
          break;
        }
        case 'campfire': {
          ctx.fillStyle = '#5A4A38'; ctx.fillRect(Math.round(sx - 10), Math.round(gy - 4), 20, 4);
          if (p.lit) {
            const fl = 4 + Math.sin(world.t * 20) * 2;
            ctx.fillStyle = '#FF8A3E'; ctx.fillRect(Math.round(sx - 3), Math.round(gy - 4 - fl), 6, fl);
          }
          break;
        }
        case 'stump': {
          ctx.fillStyle = '#6B4A2E'; ctx.fillRect(Math.round(sx - 10), Math.round(gy - 8), 20, 8);
          break;
        }
        case 'cave': {
          ctx.fillStyle = '#1A1410';
          ctx.beginPath();
          ctx.ellipse(sx, gy - (p.h as number) * 0.5, (p.w as number) * 0.5, (p.h as number) * 0.5, 0, 0, 6.283);
          ctx.fill();
          if ((!state.reduce && (p.blink as number) > 0) || p.glow) {
            ctx.fillStyle = p.glow ? '#CFFF7A' : '#fff';
            ctx.fillRect(sx - 8, gy - (p.h as number) * 0.55, 2, 2);
            ctx.fillRect(sx + 4, gy - (p.h as number) * 0.55, 2, 2);
          }
          break;
        }
        case 'scarecrow': {
          ctx.fillStyle = '#B08A4A';
          ctx.fillRect(Math.round(sx - 2), Math.round(gy - (p.h as number)), 4, p.h as number);
          const sw = (!state.reduce && (p.spin as number) > 0) ? Math.sin((p.spin as number) * 30) * 10 : 0;
          ctx.fillRect(Math.round(sx - 12 + sw), Math.round(gy - (p.h as number) * 0.7), 24, 3);
          ctx.fillStyle = '#E9C88A'; ctx.fillRect(Math.round(sx - 4), Math.round(gy - (p.h as number) - 6), 8, 8);
          if (p.feather) { ctx.fillStyle = '#E9EDF0'; ctx.fillRect(Math.round(sx + 3), Math.round(gy - (p.h as number) - 10), 2, 6); }
          break;
        }
        case 'cattails': {
          ctx.fillStyle = '#6B4A2E'; ctx.fillRect(Math.round(sx - 1), Math.round(gy - 20), 2, 20);
          ctx.fillStyle = '#5A3C24'; ctx.fillRect(Math.round(sx - 2), Math.round(gy - 24), 4, 6);
          break;
        }
        case 'boulder': {
          const bs = p.rolled ? 0 : 1;
          if (bs) {
            ctx.fillStyle = '#5E5E64';
            ctx.fillRect(Math.round(sx - (p.w as number) / 2), Math.round(gy - (p.h as number)), p.w as number, p.h as number);
          } else {
            ctx.fillStyle = '#1A1410';
            ctx.beginPath(); ctx.ellipse(sx, gy - 4, 10, 5, 0, 0, 6.283); ctx.fill();
          }
          break;
        }
        case 'signpost': {
          ctx.fillStyle = '#6B4A2E'; ctx.fillRect(Math.round(sx - 2), Math.round(gy - (p.h as number)), 4, p.h as number);
          ctx.fillStyle = '#B08A4A'; ctx.fillRect(Math.round(sx - 16), Math.round(gy - (p.h as number) + 4), 32, 10);
          break;
        }
        case 'anthill': {
          ctx.fillStyle = '#6B4A2E';
          ctx.beginPath(); ctx.ellipse(sx, gy - 4, 10, 6, 0, 0, 6.283); ctx.fill();
          break;
        }
        case 'snowpile': {
          ctx.fillStyle = '#F4F8FB';
          ctx.beginPath(); ctx.ellipse(sx, gy - (p.h as number) * 0.4, (p.w as number) * 0.5, (p.h as number) * 0.5, 0, 0, 6.283); ctx.fill();
          break;
        }
        case 'snowman': {
          ctx.fillStyle = '#F4F8FB';
          ctx.beginPath(); ctx.ellipse(sx, gy - 8, 9, 9, 0, 0, 6.283); ctx.fill();
          ctx.beginPath(); ctx.ellipse(sx, gy - 20, 6, 6, 0, 0, 6.283); ctx.fill();
          ctx.fillStyle = '#D0392B'; ctx.fillRect(sx - 1, gy - 21, 2, 2);
          if (p.waved) { ctx.fillStyle = '#6B4A2E'; ctx.fillRect(sx + 6, gy - 26, 8, 2); }
          break;
        }
        case 'star': {
          ctx.fillStyle = '#FFE47A';
          ctx.globalAlpha = 0.5 + (p.rank as number) * 0.1;
          ctx.fillRect(sx - 2, (p.y as number) - 2, 4, 4);
          ctx.globalAlpha = 1;
          break;
        }
        case 'rainbow-end': {
          if (p.side === 'l') {
            const cols = ['#E6533E', '#F4C542', '#5E9E4A', '#3EC7C0', '#63B4FF', '#C58BFF'];
            const cxx = ((p.x as number) + (p.pairX as number)) / 2 + paraX(state, 'sky');
            const rad = ((p.pairX as number) - (p.x as number)) / 2;
            for (let ri = 0; ri < cols.length; ri++) {
              ctx.strokeStyle = cols[ri];
              ctx.lineWidth = 4;
              ctx.beginPath();
              ctx.arc(cxx, gy, rad - ri * 4 + 24, Math.PI, 0);
              ctx.stroke();
            }
          }
          break;
        }
        case 'sunmoon': case 'clouds': break; // rendered by the sky pass; these are invisible hit targets
        default: break;
      }
      ctx.restore();
    },
    drawItem: function (ctx, it) {
      const s = screenOf(state, it.x, 2), bob = Math.sin(it.t * 3) * 2;
      ctx.font = '14px system-ui, sans-serif';
      ctx.textAlign = 'center';
      ctx.fillText(ITEMS[it.id] ? ITEMS[it.id].glyph : '?', s.x, s.y - 6 + bob);
    },
    drawHoverGlow: function (ctx, hit) {
      const o = hit.obj;
      let sx: number, gy: number;
      if (hit.kind === 'entity') {
        const ent = o as Entity;
        sx = ent.layer === 'sky' ? ent.x + paraX(state, 'sky') : ent.x + paraX(state, ent.layer);
        gy = ent.layer === 'sky' ? ent.y
          : ent.layer === 'pond' ? (ent.state === 'jump' ? (ent.jy || waterSurfaceY(state)) : waterSurfaceY(state))
            : groundYWorld(state, ent.x, ent.layer) + paraY(state, ent.layer);
      } else {
        const pr = o as Prop;
        sx = pr.layer === 'sky' ? pr.x + paraX(state, 'sky') : pr.x + paraX(state, pr.layer || 2);
        gy = pr.layer === 'sky' ? (pr.y !== undefined ? pr.y : state.H * 0.16)
          : pr.layer === 'pond' ? waterSurfaceY(state)
            : (pr.y !== undefined ? pr.y + paraY(state, pr.layer || 2) : groundYWorld(state, pr.x, pr.layer || 2) + paraY(state, pr.layer || 2));
      }
      const hw = ((o as { hb?: { w: number } }).hb ? (o as { hb: { w: number } }).hb.w : 30) / 2 + 6;
      const hh = ((o as { hb?: { h: number } }).hb ? (o as { hb: { h: number } }).hb.h : 30) / 2 + 8;
      ctx.save();
      ctx.strokeStyle = 'rgba(255,255,255,.75)';
      ctx.lineWidth = 2;
      ctx.shadowBlur = 8;
      ctx.shadowColor = '#fff';
      ctx.strokeRect(Math.round(sx - hw), Math.round(gy - hh * 1.6), Math.round(hw * 2), Math.round(hh * 1.6));
      ctx.restore();
    },
    draw: function (ctx) {
      const self = this;
      const order: Array<number | 'sky'> = ['sky', 0, 1, 2];
      function vis(p: Prop) { return !p.visibleIf || p.visibleIf(); }
      order.forEach(function (L) {
        self.props.filter(function (p) { return p.layer === L && vis(p); }).forEach(function (p) { self.drawProp(ctx, p); });
        self.entities.filter(function (e) { return e.layer === L; }).forEach(function (e) { self.drawEntity(ctx, e); });
      });
      this.props.filter(function (p) { return p.layer === 'pond' && vis(p); }).forEach(function (p) { self.drawProp(ctx, p); });
      this.entities.filter(function (e) { return e.layer === 'pond'; }).forEach(function (e) { self.drawEntity(ctx, e); });
      this.items.forEach(function (it) { self.drawItem(ctx, it); });
      // rain patch under a clicked cloud
      this.props.forEach(function (p) {
        if (!state.reduce && p.key === 'clouds' && (p.rainT as number) > 0) {
          ctx.strokeStyle = 'rgba(190,220,255,.5)';
          ctx.lineWidth = 1.4;
          ctx.beginPath();
          for (let i = 0; i < 8; i++) {
            const rx = (p.x as number) + paraX(state, 'sky') - 24 + i * 7;
            const ry = (p.y as number) + 16 + ((world.t * 160 + i * 23) % 80);
            ctx.moveTo(rx, ry);
            ctx.lineTo(rx - 2, ry + 10);
          }
          ctx.stroke();
        }
      });
      if (this.worldFx.length) {
        ctx.font = '12px system-ui, sans-serif';
        ctx.textAlign = 'center';
        this.worldFx.forEach(function (wf) {
          ctx.globalAlpha = Math.max(0, wf.life);
          ctx.fillStyle = '#fff';
          ctx.fillText(wf.text, wf.x, wf.y);
        });
        ctx.globalAlpha = 1;
      }
      if (this.hover) this.drawHoverGlow(ctx, this.hover);
      if (this.carry) {
        const cx = state.mx * state.W, cy = state.my * state.H;
        ctx.font = '17px system-ui, sans-serif';
        ctx.textAlign = 'center';
        ctx.fillText(ITEMS[this.carry] ? ITEMS[this.carry].glyph : '?', cx, cy - 22 + Math.sin(world.t * 4) * 2);
      }
    },
    interact: function (hit) {
      this.lastInteractAt = this.t;
      if (hit.kind === 'item') {
        const it = hit.obj as WorldItem;
        this.items.splice(this.items.indexOf(it), 1);
        this.pickUp(it.id, it.x, it.y);
        return;
      }
      if (hit.kind === 'entity') {
        const e = hit.obj as Entity, sp = e.sp;
        if (sp && sp.onClick) sp.onClick.call(sp, e);
        return;
      }
      if (hit.kind === 'prop') {
        const p = hit.obj as Prop;
        if (p.reaction) p.reaction(p);
        return;
      }
    },
    drop: function (x, _y) {
      if (!this.carry) return;
      const carried = this.carry;
      // F7: the pointer is in screen space — store the WORLD x so the item lands
      // under the cursor and egg logic compares world x to world x
      const wx = x - paraX(state, 2);
      this.items.push({ id: carried, x: wx, y: 0, t: 0 });
      if (carried === 'truffle' && this.nearPond(wx)) { this.flags.truffleThrown = true; this._findEgg?.('truffle-pond'); }
      else if (carried === 'acorn' && this.nearPond(wx)) { this.flags.acornInPond = true; dropGroundItem(state, 'coin', wx); this._findEgg?.('acorn-pond'); }
      else if (carried === 'acorn') {
        const buries = (this.flags.acornBuries = (this.flags.acornBuries as Record<string, boolean>) || {});
        buries[Math.round(wx / 60)] = true;
        if (Object.keys(buries).length >= 3) this.flags.acornHuntArmed = true;
      }
      this.dropCarry();
    },
  };

  world.emit = opts.emit;

  // The firefly-swarm species (only spawned by spawnSwarm).
  const SWARM_SP: Species = {
    key: 'swarm', name: 'Firefly swarm', kind: 'glow', pal: P('#CFFF7A', '#fff'),
    w: 0, t: 'A', layer: 2, speed: 0,
    onClick: function (e) {
      if (e.data.spelled) return;
      e.data.spelled = true;
      reactState(e, 'glow', 3);
      playVoice(state, { freqs: [880, 988, 1109, 1245, 1397, 1568, 1760], gap: 90 });
      world.popFx(e.x, e.y - 30, 'A G O R A');
      world._findEgg?.('moonlit-rave');
    },
  };

  // Bind the game API the species/props close over, then build them.
  const findEgg = makeFindEgg(state, opts.emit);
  const api: GameApi = {
    world: () => world,
    reactState,
    fleeState,
    boltState,
    exitToward,
    dropGroundItem: (itemId, x) => dropGroundItem(state, itemId, x),
    playVoice: (v) => playVoice(state, v),
    findEgg,
    shakeProp,
    blip: opts.blip,
  };
  buildSpecies(api);
  world.props = buildProps(state, api);
  world._findEgg = findEgg;
  // keep the achievement checker reachable after resume
  void checkAchievements;
  return world;
}
