/**
 * Geometric focus movement.
 *
 * Pure functions over rectangles, with no DOM access at all. That is partly
 * because geometry is the fiddly part and deserves direct tests, and partly
 * because jsdom has no layout engine: every `getBoundingClientRect()` there
 * returns zeroes, so anything that reasoned about the DOM directly would be
 * untestable in the unit suite.
 *
 * The degenerate case is load-bearing rather than defensive. When rectangles
 * carry no usable geometry — before first layout, in a hidden tab, or under
 * jsdom — `chooseCandidate` reports that it cannot decide, and the caller falls
 * back to document order. A controller that moves in source order is mildly
 * surprising; one that stops moving looks broken.
 */
import type { ControllerDirection } from './intents';

export interface NavRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface SpatialCandidate<T> {
  item: T;
  rect: NavRect;
}

/**
 * How much a candidate is punished for sitting off to one side.
 *
 * Above roughly 1 this prefers a well-aligned target further away over a nearer
 * one that is off-axis, which is what makes grid movement feel like a grid
 * rather than a scatter of nearest neighbours.
 */
const CROSS_AXIS_WEIGHT = 3;

/** Rectangles this small in both dimensions carry no usable geometry. */
const DEGENERATE_AREA = 1;

function isDegenerate(rect: NavRect): boolean {
  return (rect.right - rect.left) < DEGENERATE_AREA && (rect.bottom - rect.top) < DEGENERATE_AREA;
}

/** Overlap length of two 1-D spans; negative when they are apart. */
function overlap(aStart: number, aEnd: number, bStart: number, bEnd: number): number {
  return Math.min(aEnd, bEnd) - Math.max(aStart, bStart);
}

function centre(start: number, end: number): number {
  return (start + end) / 2;
}

interface Axis {
  /** Distance from the origin's leading edge to the candidate's near edge. */
  advance: (origin: NavRect, candidate: NavRect) => number;
  /** How far the candidate sits off the movement axis; 0 when they overlap. */
  offAxis: (origin: NavRect, candidate: NavRect) => number;
  /** Centre-to-centre separation across the movement axis, for tie-breaks. */
  centreGap: (origin: NavRect, candidate: NavRect) => number;
  /** True when the two rects genuinely share extent across the movement axis. */
  sharesExtent: (origin: NavRect, candidate: NavRect) => boolean;
}

const AXES: Record<ControllerDirection, Axis> = {
  down: {
    advance: (o, c) => c.top - o.bottom,
    offAxis: (o, c) => Math.max(0, -overlap(o.left, o.right, c.left, c.right)),
    centreGap: (o, c) => Math.abs(centre(c.left, c.right) - centre(o.left, o.right)),
    sharesExtent: (o, c) => overlap(o.left, o.right, c.left, c.right) > 0,
  },
  up: {
    advance: (o, c) => o.top - c.bottom,
    offAxis: (o, c) => Math.max(0, -overlap(o.left, o.right, c.left, c.right)),
    centreGap: (o, c) => Math.abs(centre(c.left, c.right) - centre(o.left, o.right)),
    sharesExtent: (o, c) => overlap(o.left, o.right, c.left, c.right) > 0,
  },
  right: {
    advance: (o, c) => c.left - o.right,
    offAxis: (o, c) => Math.max(0, -overlap(o.top, o.bottom, c.top, c.bottom)),
    centreGap: (o, c) => Math.abs(centre(c.top, c.bottom) - centre(o.top, o.bottom)),
    sharesExtent: (o, c) => overlap(o.top, o.bottom, c.top, c.bottom) > 0,
  },
  left: {
    advance: (o, c) => o.left - c.right,
    offAxis: (o, c) => Math.max(0, -overlap(o.top, o.bottom, c.top, c.bottom)),
    centreGap: (o, c) => Math.abs(centre(c.top, c.bottom) - centre(o.top, o.bottom)),
    sharesExtent: (o, c) => overlap(o.top, o.bottom, c.top, c.bottom) > 0,
  },
};

/** Candidate sits entirely inside the origin — a control within a bigger one. */
function isInside(origin: NavRect, candidate: NavRect): boolean {
  return candidate.left >= origin.left - 1
    && candidate.right <= origin.right + 1
    && candidate.top >= origin.top - 1
    && candidate.bottom <= origin.bottom + 1;
}

/**
 * Whether the candidate genuinely lies in the requested direction.
 *
 * Edge advance alone is too strict for real layouts, where neighbours abut or
 * overlap by a pixel of border. Requiring the *centre* to have moved as well
 * keeps a taller neighbour that merely overlaps from counting as "below".
 *
 * A candidate nested *inside* the origin is exempt from the advance test
 * entirely. Browse cards are focusable and contain their own "View Details"
 * button, so by edge distance that button is behind the card on every axis at
 * once and could never be reached — the card swallowed its own contents. Its
 * centre still says which way it lies, which is enough.
 */
function isAhead(origin: NavRect, candidate: NavRect, direction: ControllerDirection): boolean {
  const axis = AXES[direction];
  if (!isInside(origin, candidate) && axis.advance(origin, candidate) < -1) return false;

  if (direction === 'down') return centre(candidate.top, candidate.bottom) > centre(origin.top, origin.bottom);
  if (direction === 'up') return centre(candidate.top, candidate.bottom) < centre(origin.top, origin.bottom);
  if (direction === 'right') return centre(candidate.left, candidate.right) > centre(origin.left, origin.right);
  return centre(candidate.left, candidate.right) < centre(origin.left, origin.right);
}

/**
 * The best candidate in `direction`, or `null` when nothing lies that way.
 *
 * `null` deliberately does not mean "wrap around". A wrap decided by geometry
 * is disorienting — the eye expects the far side of the same row, not whatever
 * happens to be furthest away — so the caller decides what an exhausted
 * direction means: scroll the nearest scrollport, hand the intent to an
 * app-level binding, or do nothing.
 */
export function chooseCandidate<T>(
  origin: NavRect,
  candidates: ReadonlyArray<SpatialCandidate<T>>,
  direction: ControllerDirection,
): T | null {
  if (isDegenerate(origin)) return null;

  const axis = AXES[direction];
  let best: T | null = null;
  let bestAligned = false;
  let bestScore = Number.POSITIVE_INFINITY;
  let bestCentreGap = Number.POSITIVE_INFINITY;

  for (const { item, rect } of candidates) {
    if (isDegenerate(rect)) continue;
    if (!isAhead(origin, rect, direction)) continue;

    // Alignment is a hard preference, not a weight. Pressing Down in a grid
    // should reach the cell below even when something in the next column over
    // is closer as the crow flies; scoring the two together makes movement
    // depend on incidental distances and feel unpredictable. Candidates must
    // *share* extent to count as aligned — rectangles that merely abut belong
    // to the adjacent column, not this one.
    const aligned = axis.offAxis(origin, rect) === 0 && axis.sharesExtent(origin, rect);
    if (bestAligned && !aligned) continue;

    const score = Math.max(0, axis.advance(origin, rect))
      + axis.offAxis(origin, rect) * CROSS_AXIS_WEIGHT;
    const centreGap = axis.centreGap(origin, rect);

    const better = aligned !== bestAligned
      ? aligned
      : score < bestScore || (score === bestScore && centreGap < bestCentreGap);

    if (better) {
      best = item;
      bestAligned = aligned;
      bestScore = score;
      bestCentreGap = centreGap;
    }
  }

  return best;
}

/**
 * True when the whole candidate set carries no usable geometry, which is the
 * signal to fall back to document order rather than to stop moving.
 */
export function hasUsableGeometry(rects: ReadonlyArray<NavRect>): boolean {
  return rects.some((rect) => !isDegenerate(rect));
}
