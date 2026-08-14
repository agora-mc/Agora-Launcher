/**
 * Screen → canvas mapping for the zoomed/panned background.
 *
 * The zoom is a CSS transform on the canvas element, so the drawing moves but the
 * canvas coordinate space does not. Hit tests ran on raw client coordinates,
 * which meant that the moment you zoomed, every prop was clickable somewhere
 * other than where it appeared. These tests pin the inverse.
 */

import { describe, expect, it } from 'vitest';
import { viewToCanvas } from './terrain';

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
});
