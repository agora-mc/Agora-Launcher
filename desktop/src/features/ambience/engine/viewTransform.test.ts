/**
 * Screen → canvas mapping for the zoomed/panned background.
 *
 * The zoom is a CSS transform on the canvas element, so the drawing moves but the
 * canvas coordinate space does not. Hit tests ran on raw client coordinates,
 * which meant that the moment you zoomed, every prop was clickable somewhere
 * other than where it appeared. These tests pin the inverse.
 */

import { describe, expect, it } from 'vitest';
import { viewToCanvas, worldViewport, WORLD_W } from './terrain';

const W = 1000;
const H = 600;
const view = (zoom: number, tx = 0, ty = 0) => ({ zoom, tx, ty });

describe('viewToCanvas', () => {
  it('is the identity at rest', () => {
    expect(viewToCanvas(300, 200, view(1), W, H)).toEqual({ x: 300, y: 200 });
  });

  it('leaves the centre fixed — the transform origin does not move', () => {
    for (const z of [0.5, 1, 1.5, 2]) {
      expect(viewToCanvas(W / 2, H / 2, view(z), W, H)).toEqual({ x: W / 2, y: H / 2 });
    }
  });

  it('halves the offset from centre at 2x', () => {
    // A point 200px right of centre on screen is only 100px right of centre in
    // canvas space when everything is drawn twice as large.
    expect(viewToCanvas(W / 2 + 200, H / 2, view(2), W, H)).toEqual({ x: W / 2 + 100, y: H / 2 });
  });

  it('doubles the offset from centre at 0.5x', () => {
    expect(viewToCanvas(W / 2 + 100, H / 2, view(0.5), W, H)).toEqual({ x: W / 2 + 200, y: H / 2 });
  });

  it('subtracts the pan before scaling', () => {
    // Dragged 120px left: the world moved left, so the point under a fixed
    // screen position is further RIGHT in canvas space.
    const p = viewToCanvas(W / 2, H / 2, view(2, -120, 60), W, H);
    expect(p.x).toBeCloseTo(W / 2 + 60, 6);
    expect(p.y).toBeCloseTo(H / 2 - 30, 6);
  });

  it('round-trips against the forward transform for arbitrary views', () => {
    const forward = (x: number, y: number, v: { zoom: number; tx: number; ty: number }) => ({
      x: (x - W / 2) * v.zoom + W / 2 + v.tx,
      y: (y - H / 2) * v.zoom + H / 2 + v.ty,
    });
    for (const v of [view(1), view(2), view(0.5), view(1.7, -80, 45), view(1.2, 200, -130)]) {
      for (const [cx, cy] of [[0, 0], [123, 456], [W, H], [W / 2, H / 2]]) {
        const screen = forward(cx, cy, v);
        const back = viewToCanvas(screen.x, screen.y, v, W, H);
        expect(back.x).toBeCloseTo(cx, 6);
        expect(back.y).toBeCloseTo(cy, 6);
      }
    }
  });

  /**
   * Zooming out grows the canvas past the viewport and centres it, so its
   * top-left is NEGATIVE in client coordinates. That offset was missing, and
   * because the default zoom is 0.9 it was missing all the time: every hit test
   * landed tens of pixels from the cursor, further off the nearer the edge.
   *
   * The world scale is the second half of the same chain: the scene has fixed
   * borders and is drawn scaled into the element, so the element's client px
   * and the world's own units are only equal on a window exactly `WORLD_W`
   * wide. These drive the REAL `worldViewport`, not a copy of its maths.
   */
  describe('with the canvas offset and scaled from the viewport origin', () => {
    it('maps the viewport centre to the world centre at the default zoom', () => {
      const vw = 1600, vh = 900, zoom = 0.9;
      const c = worldViewport(vw, vh, zoom);
      const p = viewToCanvas(vw / 2, vh / 2, view(zoom), c.cssW, c.cssH, c.left, c.top, c.scale);
      expect(p.x).toBeCloseTo(c.cssW / c.scale / 2, 6);
      expect(p.y).toBeCloseTo(c.cssH / c.scale / 2, 6);
    });

    it('maps the viewport corners to the corners of the drawn world', () => {
      const vw = 1600, vh = 900, zoom = 0.9;
      const c = worldViewport(vw, vh, zoom);
      // Zoomed out, the canvas is grown by exactly enough that the window's
      // corners ARE the world's corners — that is what the cover buys.
      const tl = viewToCanvas(0, 0, view(zoom), c.cssW, c.cssH, c.left, c.top, c.scale);
      expect(tl.x).toBeCloseTo(0, 0);
      expect(tl.y).toBeCloseTo(0, 0);
      const br = viewToCanvas(vw, vh, view(zoom), c.cssW, c.cssH, c.left, c.top, c.scale);
      expect(br.x).toBeCloseTo(c.cssW / c.scale, 0);
      expect(br.y).toBeCloseTo(c.cssH / c.scale, 0);
    });

    it('round-trips through the full element transform', () => {
      for (const [vw, vh] of [[1600, 900], [1280, 800], [900, 1000]]) {
        for (const zoom of [0.9, 1, 1.6]) {
          const c = worldViewport(vw, vh, zoom);
          const v = view(zoom, zoom > 1 ? -90 : 0, zoom > 1 ? 40 : 0);
          // What the browser paints: world units scaled into the element, then
          // the element origin, then translate, then scale about its centre.
          const forward = (x: number, y: number) => ({
            x: c.left + v.tx + (x * c.scale - c.cssW / 2) * v.zoom + c.cssW / 2,
            y: c.top + v.ty + (y * c.scale - c.cssH / 2) * v.zoom + c.cssH / 2,
          });
          for (const [cx, cy] of [[0, 0], [321, 654], [c.W, c.H], [c.W / 2, c.H / 2]]) {
            const s = forward(cx, cy);
            const back = viewToCanvas(s.x, s.y, v, c.cssW, c.cssH, c.left, c.top, c.scale);
            expect(back.x).toBeCloseTo(cx, 6);
            expect(back.y).toBeCloseTo(cy, 6);
          }
        }
      }
    });
  });
});

/**
 * The world's size against the window's.
 *
 * The canvas WAS the viewport, so `state.W` was the window's width: `ridge`
 * generates one column per step across `W`, so widening the window appended
 * hills past the old edge, and everything placed at a fraction of `W` — the
 * sun's arc, the cloud wrap, the spawns — spread apart with it. Dragging the
 * window wider grew the map instead of enlarging the view of it. The world is
 * a fixed-width place now; the window only changes `scale`.
 */
describe('worldViewport', () => {
  const WIDTHS = [900, 1280, 1600, 1920, 2560, 3440];

  it('keeps the world the same width whatever the window does', () => {
    for (const zoom of [0.9, 1, 1.5, 2]) {
      const widths = WIDTHS.map((vw) => worldViewport(vw, 900, zoom).W);
      expect(new Set(widths).size).toBe(1);
    }
  });

  it('is the world at 1:1 on the app\'s own default window width', () => {
    const c = worldViewport(WORLD_W, 800, 1);
    expect(c.scale).toBe(1);
    expect(c.W).toBe(WORLD_W);
    expect(c.H).toBe(800);
  });

  it('scales instead of growing: twice the window, twice the world on screen', () => {
    const a = worldViewport(1280, 800, 1);
    const b = worldViewport(2560, 800, 1);
    expect(b.W).toBe(a.W);
    expect(b.scale).toBeCloseTo(a.scale * 2, 6);
  });

  it('never stretches — one scale for both axes', () => {
    for (const [vw, vh] of [[1600, 900], [900, 1000], [3440, 1440], [1280, 800]]) {
      for (const zoom of [0.9, 1, 2]) {
        const c = worldViewport(vw, vh, zoom);
        expect(c.cssH / c.H).toBeCloseTo(c.cssW / c.W, 1);
        expect(c.cssW / c.W).toBeCloseTo(c.scale, 1);
      }
    }
  });

  it('gives a taller window more sky rather than a taller world', () => {
    // Same width, more height: the extra arrives as world units ABOVE the
    // ground, which `ridgeBase` then hands to the sky.
    const short = worldViewport(1600, 700, 1);
    const tall = worldViewport(1600, 1200, 1);
    expect(tall.W).toBe(short.W);
    expect(tall.H).toBeGreaterThan(short.H);
    expect(tall.scale).toBe(short.scale);
  });

  it('covers the viewport exactly at zoom >= 1', () => {
    for (const zoom of [1, 1.4, 2]) {
      const c = worldViewport(1600, 900, zoom);
      expect(c.cssW).toBe(1600);
      expect(c.cssH).toBe(900);
      expect(c.left).toBe(0);
      expect(c.top).toBe(0);
    }
  });

  it('grows and re-centres the canvas when zoomed out, so no gap can show', () => {
    const c = worldViewport(1600, 900, 0.9);
    // Scaled by 0.9 the grown element still covers the window...
    expect(c.cssW * 0.9).toBeGreaterThanOrEqual(1600);
    expect(c.cssH * 0.9).toBeGreaterThanOrEqual(900);
    // ...and it is centred, which is where the negative origin comes from.
    expect(c.left).toBeLessThan(0);
    expect(c.top).toBeLessThan(0);
    expect(c.left).toBeCloseTo((1600 - c.cssW) / 2, 0);
    // The zoom, not the window, is what reveals more world.
    expect(c.W).toBeGreaterThan(worldViewport(1600, 900, 1).W);
  });
});
