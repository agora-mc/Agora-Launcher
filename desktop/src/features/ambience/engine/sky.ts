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
import { carveBasin, ridge, waterSpan } from './terrain';
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

export function regenerateTerrain(state: EngineState): void {
  if (state.W === state.lastW) return;
  state.lastW = state.W;
  state.R1 = ridge(1.3, 90, state.H * 0.62, 16, state.W, state.OVER);
  state.R2 = ridge(4.7, 70, state.H * 0.74, 14, state.W, state.OVER);
  state.R3 = ridge(8.1, 44, state.H * 0.86, 12, state.W, state.OVER);
  state.BASIN = carveBasin(state.R3, 12, 0.30, 11, 46);
  // water fills the bowl up to its lowest rim — stays in the dip, never floods
  const b = state.BASIN;
  state.WATER_LEVEL = Math.max(state.R3[b.c - b.half], state.R3[b.c + b.half]);
  reanchorPond(state);
}

/** Render exactly one background frame at time `ts`. */
export function skyFrame(state: EngineState, ctx: CanvasRenderingContext2D, ts: number): void {
  if (state.W !== state.lastW) {
    state.lastW = state.W;
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
