/**
 * The living background frame — sky gradient, day/night, stars, sun/moon,
 * clouds, parallax ridges, grass strip, weather, and the world's own update
 * and draw. Ported verbatim from v4-world.html's `bgFrame`.
 *
 * The engine owns the requestAnimationFrame loop; `skyFrame` renders exactly
 * one frame. `document.hidden` pauses everything (the update loop, the music
 * context, and rAF all stop — prototype trap 2).
 */

import type { EngineState } from './state';
import { mixc, rgba } from './state';
import { basinColumn, carveBasin, groundYWorld, paraX, ridge, ridgeBase, waterSpan } from './terrain';
import type { WorldState } from './types';

/**
 * F9: water sits in a carved valley. The level and span derive from the basin
 * once the terrain is generated. The pond and lily pads are anchored to the
 * ACTUAL water after the terrain exists (the ridge is wavy, so the submerged
 * span isn't a fixed screen x); on resize the terrain regenerates, so this
 * re-anchoring keeps the hitbox, nearPond and the fish spawn aligned.
 */
function reanchorPond(state: EngineState): void {
  const world = state.world as WorldState | null;
  if (!world || !world.props) return;
  const ws = waterSpan(state);
  if (ws && ws.x1 > ws.x0) {
    const pondP = world.pondProp();
    if (pondP) pondP.x = (ws.x0 + ws.x1) / 2;
    world.props.filter(function (p) { return p.key === 'lily'; }).forEach(function (lp) { lp.x = (ws.x0 + ws.x1) / 2; });
  }
}

/**
 * F-rainbow-layer: the rainbow is drawn in the SKY pass (after the clouds,
 * before the parallax ridges) so it sits just in front of the sky and BEHIND
 * the terrain. The rainbow-end props are invisible hit targets (their `y` is
 * anchored to the terrain so the arc's flat ends read as rising from behind
 * the hills).
 */
function drawRainbow(state: EngineState, ctx: CanvasRenderingContext2D): void {
  const world = state.world as WorldState | null;
  if (!world || !world.props) return;
  const left = world.props.find(function (p) { return p.key === 'rainbow-end' && p.side === 'l'; });
  if (!left) return;
  const cols = ['#E6533E', '#F4C542', '#5E9E4A', '#3EC7C0', '#63B4FF', '#C58BFF'];
  const cxx = ((left.x as number) + (left.pairX as number)) / 2 + paraX(state, 'sky');
  const rad = ((left.pairX as number) - (left.x as number)) / 2;
  const gy = left.y !== undefined ? left.y : state.H * 0.55;
  for (let ri = 0; ri < cols.length; ri++) {
    ctx.strokeStyle = cols[ri];
    ctx.lineWidth = 4;
    ctx.beginPath();
    ctx.arc(cxx, gy, rad - ri * 4 + 24, Math.PI, 0);
    ctx.stroke();
  }
}

/**
 * Rebuild the ridges, the basin and the water for the current canvas size.
 *
 * Resizing must move the world as little as possible. `ridge` is deterministic
 * per column, so a wider canvas only APPENDS hills — the existing ones keep
 * their shape as long as their baseline does, which is what `ridgeBase` (fixed
 * distance above the bottom edge) buys. And the basin is carved around a world
 * x pinned on the first pass, not around a fraction of the ridge array, whose
 * length tracks the width: that fraction is why the pond used to slide sideways
 * every time the window changed shape.
 */
export function regenerateTerrain(state: EngineState): void {
  if (state.W === state.lastW && state.H === state.lastH) return;
  state.lastW = state.W;
  state.lastH = state.H;
  if (state.refH === 0) state.refH = state.H;
  const [s1, s2, s3] = state.ridgeSeeds;
  state.R1 = ridge(s1, 90, ridgeBase(state.H, state.refH, 0.62), 16, state.W, state.OVER);
  state.R2 = ridge(s2, 70, ridgeBase(state.H, state.refH, 0.74), 14, state.W, state.OVER);
  state.R3 = ridge(s3, 44, ridgeBase(state.H, state.refH, 0.86), 12, state.W, state.OVER);
  const col = basinColumn(state.R3, state.basinAnchorX, 0.30, 12, state.OVER);
  if (state.basinAnchorX === null) state.basinAnchorX = col * 12 - state.OVER;
  state.BASIN = carveBasin(state.R3, 12, col, 11, 46);
  // water fills the bowl up to its lowest rim — stays in the dip, never floods
  const b = state.BASIN;
  state.WATER_LEVEL = Math.max(state.R3[b.c - b.half], state.R3[b.c + b.half]);
  reanchorPond(state);
  reanchorRainbow(state);
}

/**
 * Re-span the rainbow across the new viewport.
 *
 * Its ends were positioned once, in absolute pixels, at spawn time — so after a
 * resize the arc kept the old window's width and drifted away from the hills it
 * is supposed to rise from.
 */
export function reanchorRainbow(state: EngineState): void {
  const world = state.world as WorldState | null;
  if (!world || !world.props) return;
  for (const p of world.props) {
    if (p.key !== 'rainbow-end') continue;
    const fx = p.fx as number | undefined;
    const pairFx = p.pairFx as number | undefined;
    if (fx === undefined || pairFx === undefined) continue;
    p.x = state.W * fx;
    p.pairX = state.W * pairFx;
  }
  // Both ends sit on the lower of the two ground heights, as at spawn.
  const l = world.props.find((p) => p.key === 'rainbow-end' && p.side === 'l');
  const r = world.props.find((p) => p.key === 'rainbow-end' && p.side === 'r');
  if (l && r) {
    const gy = Math.max(
      groundYWorld(state, l.x as number, 2),
      groundYWorld(state, r.x as number, 2),
    );
    l.y = gy;
    r.y = gy;
  }
}

/** Render exactly one background frame at time `ts`. */
export function skyFrame(state: EngineState, ctx: CanvasRenderingContext2D, ts: number): void {
  // regenerateTerrain guards on the size itself (and records it after
  // generating); do NOT pre-set lastW/lastH here or the guard always
  // early-returns and the terrain (ridges, basin, water) never renders
  // (F-terrain).
  if (state.W !== state.lastW || state.H !== state.lastH) {
    regenerateTerrain(state);
  }
  const day = state.tod; // 0..1  (0 = deep night, .5 = noon)
  const dl = Math.max(0, Math.sin(day * Math.PI));
  const top = mixc([8, 14, 34], [74, 150, 220], dl), bot = mixc([26, 34, 60], [176, 218, 238], dl);
  const g = ctx.createLinearGradient(0, 0, 0, state.H);
  g.addColorStop(0, rgba(top, 1));
  g.addColorStop(1, rgba(bot, 1));
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, state.W, state.H);
  if (state.flash > 0) {
    ctx.fillStyle = 'rgba(210,230,255,' + (state.flash * 0.5) + ')';
    ctx.fillRect(0, 0, state.W, state.H);
    state.flash -= 0.06;
  }

  if (dl < 0.42) {
    const sa = (1 - dl / 0.42);
    state.stars.forEach(function (s) {
      ctx.globalAlpha = sa * (0.35 + 0.65 * Math.abs(Math.sin(ts / 700 + s.p)));
      ctx.fillStyle = '#fff';
      ctx.fillRect(s.x * state.W + (state.mx - 0.5) * -8, s.y * state.H + (state.my - 0.5) * -6, s.r, s.r);
    });
    ctx.globalAlpha = 1;
  }

  // sun / moon
  const ang = day * Math.PI, sx = state.W * (1 - day), sy = state.H * 0.72 - Math.sin(ang) * state.H * 0.6;
  ctx.save();
  ctx.shadowBlur = 44;
  ctx.shadowColor = dl > 0.4 ? '#FFD84D' : '#DDE7FF';
  ctx.fillStyle = dl > 0.4 ? '#FFE47A' : '#E8EEFF';
  ctx.fillRect(sx - 16, sy - 16, 32, 32);
  ctx.restore();

  // clouds (parallax)
  ctx.globalAlpha = 0.16 + 0.3 * dl;
  for (let c = 0; c < 9; c++) {
    const cw = 110 + ((c * 57) % 150);
    const cx = ((ts / 38 + c * 260) % (state.W + 420)) - 210 + (state.mx - 0.5) * -30;
    const cy = 60 + ((c * 97) % 180) + (state.my - 0.5) * -18;
    ctx.fillStyle = '#fff';
    ctx.fillRect(cx, cy, cw, 17);
    ctx.fillRect(cx + 26, cy - 13, cw * 0.62, 15);
  }
  ctx.globalAlpha = 1;

  // rainbow — drawn here, in the SKY pass, so it sits just in front of the
  // sky and BEHIND the terrain ridges (F-rainbow-layer). The rainbow-end
  // props remain invisible hit targets for the egg.
  drawRainbow(state, ctx);

  // parallax ridges, far to near
  [[state.R1, mixc([16, 26, 46], [86, 124, 150], dl), 16, -26],
   [state.R2, mixc([14, 30, 38], [62, 110, 96], dl), 14, -16],
   [state.R3, mixc([12, 34, 30], [52, 120, 64], dl), 12, -8]]
    .forEach(function (L) {
      const pts = L[0] as number[] | null; if (!pts) return;
      const off = (state.mx - 0.5) * (L[3] as number) - state.OVER, yo = (state.my - 0.5) * (L[3] as number) * 0.4;
      ctx.fillStyle = rgba(L[1] as number[], 1);
      for (let i = 0; i < pts.length; i++) ctx.fillRect(i * (L[2] as number) + off, pts[i] + yo, (L[2] as number) + 1, state.H - pts[i] + state.OVER);
    });

  // grass top strip on nearest ridge
  if (state.R3) {
    const off = (state.mx - 0.5) * -8 - state.OVER, yo = (state.my - 0.5) * -3.2;
    ctx.fillStyle = rgba(mixc([22, 58, 34], [110, 190, 74], dl), 1);
    for (let i = 0; i < state.R3.length; i++) ctx.fillRect(i * 12 + off, state.R3[i] + yo, 13, 12);
  }

  // living world: dwells between the terrain and the weather so rain/snow
  // still falls in front of animals
  const dtw = Math.min((ts - state.lastTs) / 1000, 0.05) || 0;
  state.lastTs = ts;
  const world = state.world as WorldState | null;
  if (!document.hidden && world) {
    world.dl = dl;
    world.update(dtw);
    world.draw(ctx);
  }

  // weather
  if (state.WEATHER[state.weather] === 'rain') {
    ctx.strokeStyle = 'rgba(190,220,255,.42)';
    ctx.lineWidth = 1.6;
    ctx.beginPath();
    state.drops.forEach(function (d) {
      d.y += 0.016 * d.s;
      if (d.y > 1) d.y -= 1;
      const X = d.x * state.W + (state.mx - 0.5) * -14, Y = d.y * state.H;
      ctx.moveTo(X, Y);
      ctx.lineTo(X - 3, Y + 15 * d.s);
    });
    ctx.stroke();
    if (Math.random() < 0.004) state.flash = 1;
  } else if (state.WEATHER[state.weather] === 'snow') {
    state.flakes.forEach(function (f) {
      f.y += 0.0016 * f.s;
      f.x += Math.sin(ts / 1400 + f.p) * 0.0006;
      if (f.y > 1) f.y -= 1;
      ctx.globalAlpha = 0.55 + 0.4 * f.s;
      ctx.fillStyle = '#fff';
      ctx.fillRect(f.x * state.W + (state.mx - 0.5) * -10, f.y * state.H, 3 * f.s + 1, 3 * f.s + 1);
    });
    ctx.globalAlpha = 1;
  }
  if (dl < 0.35) {
    state.fireflies.forEach(function (f) {
      const a = (1 - dl / 0.35) * (0.35 + 0.65 * Math.abs(Math.sin(ts / 560 + f.p)));
      ctx.globalAlpha = a;
      ctx.fillStyle = '#CFFF7A';
      ctx.shadowBlur = 10;
      ctx.shadowColor = '#CFFF7A';
      ctx.fillRect((f.x + Math.sin(ts / 2600 + f.p) * 0.02) * state.W, (f.y + Math.cos(ts / 3100 + f.p) * 0.012) * state.H, 3, 3);
    });
    ctx.globalAlpha = 1;
    ctx.shadowBlur = 0;
  }
}
