import { describe, expect, it } from 'vitest';
import { chooseCandidate, hasUsableGeometry, type NavRect, type SpatialCandidate } from './spatialNavigation';

function rect(left: number, top: number, width = 100, height = 40): NavRect {
  return { left, top, right: left + width, bottom: top + height };
}

function named(entries: Record<string, NavRect>): SpatialCandidate<string>[] {
  return Object.entries(entries).map(([item, r]) => ({ item, rect: r }));
}

/** A 3x3 grid of 100x40 cells with 20px gutters, origin at (0, 0). */
const grid = named({
  a: rect(0, 0), b: rect(120, 0), c: rect(240, 0),
  d: rect(0, 60), e: rect(120, 60), f: rect(240, 60),
  g: rect(0, 120), h: rect(120, 120), i: rect(240, 120),
});

describe('chooseCandidate on a grid', () => {
  it('moves to the neighbour directly below', () => {
    expect(chooseCandidate(rect(120, 0), grid, 'down')).toBe('e');
  });

  it('moves to the neighbour directly above', () => {
    expect(chooseCandidate(rect(120, 120), grid, 'up')).toBe('e');
  });

  it('moves to the neighbour directly right', () => {
    expect(chooseCandidate(rect(0, 60), grid, 'right')).toBe('e');
  });

  it('moves to the neighbour directly left', () => {
    expect(chooseCandidate(rect(240, 60), grid, 'left')).toBe('e');
  });

  it('prefers a column-aligned target over a nearer diagonal one', () => {
    // `b` sits directly below the origin but further away; `d` is closer as the
    // crow flies but a column to the left. Grid movement must pick `b`.
    const candidates = named({ b: rect(100, 200), d: rect(0, 120) });

    expect(chooseCandidate(rect(100, 0), candidates, 'down')).toBe('b');
  });

  it('returns null at the edge rather than wrapping', () => {
    expect(chooseCandidate(rect(120, 120), grid, 'down')).toBeNull();
    expect(chooseCandidate(rect(0, 0), grid, 'left')).toBeNull();
  });

  it('never selects the origin itself', () => {
    expect(chooseCandidate(rect(0, 0), grid, 'right')).not.toBe('a');
  });
});

describe('chooseCandidate on awkward layouts', () => {
  it('does not treat a merely-overlapping neighbour as being below', () => {
    // A tall sidebar overlapping the origin's row is beside it, not under it.
    const candidates = named({ sidebar: rect(200, -50, 80, 200) });

    expect(chooseCandidate(rect(0, 0), candidates, 'down')).toBeNull();
    expect(chooseCandidate(rect(0, 0), candidates, 'right')).toBe('sidebar');
  });

  it('tolerates neighbours that abut exactly', () => {
    const candidates = named({ below: rect(0, 40) });

    expect(chooseCandidate(rect(0, 0), candidates, 'down')).toBe('below');
  });

  it('tolerates a one-pixel border overlap', () => {
    const candidates = named({ below: rect(0, 39) });

    expect(chooseCandidate(rect(0, 0), candidates, 'down')).toBe('below');
  });

  it('picks the nearest of several stacked rows', () => {
    const candidates = named({ near: rect(0, 60), far: rect(0, 300) });

    expect(chooseCandidate(rect(0, 0), candidates, 'down')).toBe('near');
  });

  it('breaks ties by centre alignment', () => {
    // Both are the same distance below; the better-centred one wins.
    const candidates = named({ offset: rect(60, 60), centred: rect(0, 60) });

    expect(chooseCandidate(rect(0, 0), candidates, 'down')).toBe('centred');
  });
});

describe('degenerate geometry', () => {
  const zero: NavRect = { left: 0, top: 0, right: 0, bottom: 0 };

  it('cannot decide when the origin has no layout', () => {
    expect(chooseCandidate(zero, grid, 'down')).toBeNull();
  });

  it('ignores candidates that have no layout', () => {
    const candidates: SpatialCandidate<string>[] = [
      { item: 'unlaid', rect: zero },
      { item: 'real', rect: rect(0, 60) },
    ];

    expect(chooseCandidate(rect(0, 0), candidates, 'down')).toBe('real');
  });

  it('reports an all-degenerate set so the caller can fall back to document order', () => {
    expect(hasUsableGeometry([zero, zero])).toBe(false);
    expect(hasUsableGeometry([zero, rect(0, 0)])).toBe(true);
    expect(hasUsableGeometry([])).toBe(false);
  });

  it('handles an empty candidate list', () => {
    expect(chooseCandidate(rect(0, 0), [], 'down')).toBeNull();
  });
});
