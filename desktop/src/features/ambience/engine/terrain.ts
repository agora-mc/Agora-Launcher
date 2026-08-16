/**
 * Terrain: parallax ridges, the carved basin, and the ground-column math.
 *
 * The comments here are not decoration — they record why the code is shaped
 * that way, and most of them exist because something was wrong first.
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
export function carveBasin(arr: number[], step: number, c: number, widthCols: number, depth: number): Basin {
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

/**
 * Baseline y for a ridge layer, given the session's reference height.
 *
 * A ridge used to sit at a fraction of the CURRENT height, so every resize
 * re-shaped the whole landscape: the hills, the shoreline and the water level
 * all slid vertically. Anchor them to the BOTTOM edge instead — the horizon
 * keeps its distance from the ground you can see, and a taller window simply
 * gains sky.
 *
 * The `Math.max` is the floor for windows SHORTER than the reference: below
 * that, holding the offset would push the ridges off the top of the screen, so
 * the original proportional placement takes over.
 */
export function ridgeBase(H: number, refH: number, frac: number): number {
  return Math.max(H * frac, H - refH * (1 - frac));
}

/** Basin column for a ridge array, pinned to a world x once it is chosen. */
export function basinColumn(arr: number[], anchorX: number | null, centerFrac: number, step: number, OVER: number): number {
  const c = anchorX === null ? Math.floor(arr.length * centerFrac) : Math.floor((anchorX + OVER) / step);
  return Math.max(0, Math.min(arr.length - 1, c));
}

/**
 * The world's own width in world units — the scene's standard borders.
 *
 * The canvas used to BE the viewport, so the world's coordinate space grew and
 * shrank with the window: `ridge` generates one column per `step` across `W`,
 * so a wider window APPENDED hills, and everything placed against `W` (the
 * sun's arc, the clouds' wrap, every spawn at a fraction of the width) spread
 * out with it. Widening the window enlarged the map instead of enlarging the
 * view of it.
 *
 * So the world is a fixed-size place, drawn in world units against this
 * constant and scaled to whatever window it lands in (`worldViewport`). A
 * resize changes only how big the world looks — never how much of it there is.
 * 1280 is the app's own default window width (`tauri.conf.json`), so the
 * ordinary case is a 1:1 scale.
 */
export const WORLD_W = 1280;

/** How the fixed-width world maps onto one window. See `worldViewport`. */
export interface WorldViewport {
  /** Size of the canvas ELEMENT, in client px (what the browser lays out). */
  cssW: number;
  cssH: number;
  /** The element's untransformed top-left, in client px. Negative when zoomed out. */
  left: number;
  top: number;
  /** Client px per world unit. */
  scale: number;
  /** The world's own drawing space, in world units. */
  W: number;
  H: number;
}

/**
 * Fit the fixed-width world to a window.
 *
 * `W` follows only the zoom, never the window: that is the whole point — the
 * scene keeps its borders and `scale` grows instead, so a wider window shows
 * the same world, larger.
 *
 * The vertical axis is deliberately NOT fitted the same way. Forcing a fixed
 * height too would mean either letterboxing the background or cropping the
 * ground out of a wide window, so `H` keeps the window's aspect in world units
 * and the surplus becomes sky (`ridgeBase`) — the rule the landscape already
 * followed when the window grew taller.
 *
 * Zooming OUT scales the element below the viewport and would letterbox the
 * world, so the element is grown by 1/zoom and re-centred first; that is why
 * `left`/`top` can be negative. It does not touch `scale`, so zoom stays a pure
 * zoom rather than a second, hidden resize.
 */
export function worldViewport(vw: number, vh: number, zoom: number, worldW = WORLD_W): WorldViewport {
  const cover = zoom < 1 ? 1 / zoom : 1;
  const cssW = Math.round(Math.max(1, vw) * cover);
  const cssH = Math.round(Math.max(1, vh) * cover);
  // `W` first, from the world's own constant, and `scale` from that — deriving
  // `W` from the rounded element size instead let it wobble by a pixel between
  // window widths, which is a small lie about a fixed-size world.
  const W = Math.round(worldW * cover);
  const scale = cssW / W;
  return {
    cssW,
    cssH,
    left: Math.round((vw - cssW) / 2),
    top: Math.round((vh - cssH) / 2),
    scale,
    W,
    // Ceil: the drawn height must never fall a fraction short of the element,
    // or the bottom row of pixels goes unpainted.
    H: Math.ceil(cssH / scale),
  };
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
 * Undo the view transform: screen point → world point.
 *
 * The zoom/pan is a CSS transform on the canvas element, so the pixels move but
 * the canvas's own coordinate space does not. Hit tests must therefore be run on
 * the inverse, or everything becomes clickable somewhere other than where it is
 * drawn — which is exactly what happened as soon as you zoomed.
 *
 * Mirrors `transform: translate(tx, ty) scale(zoom)` with a 50%/50% origin,
 * about an element whose untransformed top-left sits at (`left`, `top`) in
 * client coordinates. That offset is NOT always zero: zooming out grows the
 * canvas past the viewport and centres it with a negative left/top, so leaving
 * it out put every hit test tens of pixels away from the cursor at the default
 * zoom.
 *
 * `w`/`h` are the ELEMENT's untransformed size and `scale` the client px per
 * world unit (`worldViewport`) — the fixed-width world is drawn scaled, so the
 * two stopped being the same number the moment the window was not exactly
 * `WORLD_W` wide.
 */
export function viewToCanvas(
  clientX: number,
  clientY: number,
  view: { zoom: number; tx: number; ty: number },
  w: number,
  h: number,
  left = 0,
  top = 0,
  scale = 1,
): { x: number; y: number } {
  return {
    x: ((clientX - left - view.tx - w / 2) / view.zoom + w / 2) / scale,
    y: ((clientY - top - view.ty - h / 2) / view.zoom + h / 2) / scale,
  };
}
