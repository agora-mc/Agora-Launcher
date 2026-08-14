/**
 * Terrain: parallax ridges, the carved basin, and the ground-column math.
 *
 * Ported 1:1 from `v4-world.html` (sections "living background" and "engine
 * core"). The comments are not decoration — they record why the code is
 * shaped that way, and most of them exist because something was wrong first.
 *
 * F1: `Math.floor`, never `Math.round`, for the ground column index.
 * F7: `wx` is a WORLD x; screen x is derived only at draw/hit time via
 *     `screenOf`, so entities and terrain can never drift apart under mouse
 *     parallax.
 * F9: the pond lives in a carved valley; the water level and span derive
 *     from the basin, never a fixed ellipse.
 */

import type { EngineState, Basin } from './state';

/** Terrain is overscanned past both edges by OVER px so the parallax shift can
 * never drag a layer inward and expose the sky behind it. */
export function ridge(seed: number, amp: number, base: number, step: number, W: number, OVER: number): number[] {
  const pts: number[] = [], n = Math.ceil((W + OVER * 2) / step) + 2;
  for (let i = 0; i < n; i++) {
    const v = Math.sin(i * 0.5 + seed) * 0.5 + Math.sin(i * 0.17 + seed * 2) * 0.5;
    pts.push(base - Math.round(v * amp / step) * step);
  }
  return pts;
}

/**
 * F9: water sits in a carved valley, not on a hill. The level and span derive
 * from the basin once the terrain is generated. BASIN remembers the carved
 * bowl so the water (and its creatures) stay bounded to it — the ridge is too
 * wavy to trust a single rim sample, so we take the LOWER rim (max y).
 */
export function carveBasin(arr: number[], step: number, centerFrac: number, widthCols: number, depth: number): Basin {
  const c = Math.floor(arr.length * centerFrac);
  for (let i = c - widthCols; i <= c + widthCols; i++) {
    if (i < 0 || i >= arr.length) continue;
    const d = Math.cos((i - c) / widthCols * Math.PI / 2); // smooth bowl, 0 at rim
    arr[i] += Math.round(depth * d * d / step) * step;     // keep it quantised to the grid
  }
  return { c: c, half: widthCols };
}

/** F7: mouse-parallax offset for a layer (world -> screen). 'pond' uses the
 * near-ground parallax (anything not 0/1/'sky' falls through to -8). */
export function paraMul(layer: number | 'sky' | 'pond'): number {
  return layer === 0 ? -26 : layer === 1 ? -16 : layer === 'sky' ? -30 : -8;
}
export function stepFor(layer: number | 'sky' | 'pond'): number {
  return layer === 0 ? 16 : layer === 1 ? 14 : 12;
}
export function paraX(state: EngineState, layer: number | 'sky' | 'pond'): number {
  return (state.mx - 0.5) * paraMul(layer);
}
export function paraY(state: EngineState, layer: number | 'sky' | 'pond'): number {
  return (state.my - 0.5) * paraMul(layer) * 0.4;
}

/** Regenerate the terrain whenever the canvas width changes (as in bgFrame). */
export function regenerate(state: EngineState): void {
  if (state.W === state.lastW) return;
  state.lastW = state.W;
  state.R1 = ridge(1.3, 90, state.H * 0.62, 16, state.W, state.OVER);
  state.R2 = ridge(4.7, 70, state.H * 0.74, 14, state.W, state.OVER);
  state.R3 = ridge(8.1, 44, state.H * 0.86, 12, state.W, state.OVER);
  state.BASIN = carveBasin(state.R3, 12, 0.30, 11, 46);
  // water fills the bowl up to its lowest rim — stays in the dip, never floods
  const b = state.BASIN;
  state.WATER_LEVEL = Math.max(state.R3[b.c - b.half], state.R3[b.c + b.half]);
}

/**
 * F1: ground column index for a WORLD x. `Math.floor`, never `Math.round` —
 * `Math.round` was the original bug, wrong on 33% of positions by 12-24px.
 */
export function groundYWorld(state: EngineState, wx: number, layer: number | 'sky' | 'pond'): number {
  const arr = layer === 0 ? state.R1 : layer === 1 ? state.R2 : state.R3;
  if (!arr) return state.H * 0.86;
  const i = Math.floor((wx + state.OVER) / stepFor(layer));
  const clamped = Math.max(0, Math.min(arr.length - 1, i));
  return arr[clamped];
}

/** Screen position of anything tethered to the ground (F7). */
export function screenOf(state: EngineState, wx: number, layer: number | 'sky' | 'pond'): { x: number; y: number } {
  return { x: wx + paraX(state, layer), y: groundYWorld(state, wx, layer) + paraY(state, layer) };
}

/** F9: the pond's water surface (a level, with the mouse parallax applied). */
export function waterSurfaceY(state: EngineState): number {
  return state.WATER_LEVEL + paraY(state, 2);
}

/** F9: world-x range of the submerged columns INSIDE the basin. */
export function waterSpan(state: EngineState): { x0: number; x1: number } {
  if (!state.BASIN || !state.R3) return { x0: 0, x1: 1 };
  const b = state.BASIN;
  let a: number | null = null, z: number | null = null;
  for (let i = b.c - b.half; i <= b.c + b.half; i++) {
    if (i < 0 || i >= state.R3.length) continue;
    if (state.R3[i] > state.WATER_LEVEL) { if (a === null) a = i; z = i; }
  }
  if (a === null) return { x0: 0, x1: 1 };
  return { x0: (a * 12) - state.OVER, x1: ((z as number) * 12) - state.OVER };
}

/**
 * Undo the view transform: screen point → canvas point.
 *
 * The zoom/pan is a CSS transform on the canvas element, so the pixels move but
 * the canvas's own coordinate space does not. Hit tests must therefore be run on
 * the inverse, or everything becomes clickable somewhere other than where it is
 * drawn — which is exactly what happened as soon as you zoomed.
 *
 * Mirrors `transform: translate(tx, ty) scale(zoom)` with a 50%/50% origin.
 */
export function viewToCanvas(
  clientX: number,
  clientY: number,
  view: { zoom: number; tx: number; ty: number },
  w: number,
  h: number,
): { x: number; y: number } {
  return {
    x: (clientX - view.tx - w / 2) / view.zoom + w / 2,
    y: (clientY - view.ty - h / 2) / view.zoom + h / 2,
  };
}
