/**
 * Reduced motion holds the living world still (one painted frame, no frame
 * loop) and the cursor companion steps aside over controls.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AmbienceEngine, buddyFootprint, coversContent } from './engine/engine';

type EnginePrivate = { paintStill: () => void; buddy: { on: boolean } };

let rafQueue: Array<FrameRequestCallback> = [];
let engines: AmbienceEngine[] = [];
const originalGetContext = HTMLCanvasElement.prototype.getContext;

beforeEach(() => {
  const makeCtx = () => {
    const target: Record<string, unknown> = {};
    return new Proxy(target, {
      get(t, prop) {
        if (prop === 'canvas') return { width: 0, height: 0 };
        if (prop in t) return t[prop as string];
        if (prop === 'createLinearGradient' || prop === 'createRadialGradient' || prop === 'createPattern') {
          return () => ({ addColorStop: () => undefined });
        }
        if (prop === 'measureText') return () => ({ width: 0 });
        if (prop === 'getImageData') return () => ({ data: [] });
        return () => undefined;
      },
      set(t, prop, val) { t[prop as string] = val; return true; },
    });
  };
  HTMLCanvasElement.prototype.getContext = function (contextId: string) {
    if (contextId !== '2d') return null;
    return makeCtx() as unknown as CanvasRenderingContext2D;
  } as typeof HTMLCanvasElement.prototype.getContext;
  // Frames are queued, not run, so each test decides when a frame happens.
  rafQueue = [];
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation((cb) => {
    rafQueue.push(cb);
    return rafQueue.length;
  });
  vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(() => undefined);
  engines = [];
});

afterEach(() => {
  engines.forEach((e) => e.stop());
  HTMLCanvasElement.prototype.getContext = originalGetContext;
  vi.restoreAllMocks();
  // Each test that overrides elementFromPoint defines it on the instance.
  delete (document as unknown as Record<string, unknown>).elementFromPoint;
});

function mountEngine(options: { reducedMotion: boolean; profile?: 'calm' | 'full' }): AmbienceEngine {
  const engine = new AmbienceEngine(
    document.createElement('canvas'),
    document.createElement('canvas'),
    { profile: options.profile ?? 'full', musicOn: false, reducedMotion: options.reducedMotion },
  );
  engines.push(engine);
  engine.start();
  return engine;
}

function flushFrames(): void {
  const pending = rafQueue;
  rafQueue = [];
  pending.forEach((cb) => cb(0));
}

function paintSpy() {
  return vi.spyOn(AmbienceEngine.prototype as unknown as EnginePrivate, 'paintStill');
}

function companionOn(engine: AmbienceEngine): boolean {
  return (engine as unknown as EnginePrivate).buddy.on;
}

function pointAt(element: Element): void {
  Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: () => element });
}

describe('AmbienceEngine reduced motion', () => {
  it('paints one still frame and runs no frame loop', () => {
    const paint = paintSpy();
    mountEngine({ reducedMotion: true });
    expect(paint).toHaveBeenCalledTimes(1);
    flushFrames();
    expect(rafQueue).toHaveLength(0);
    expect(paint).toHaveBeenCalledTimes(1);
  });

  it('runs the frame loop without reduced motion and paints no still frame', () => {
    const paint = paintSpy();
    mountEngine({ reducedMotion: false });
    expect(paint).not.toHaveBeenCalled();
    expect(rafQueue).toHaveLength(1);
  });

  it('switching reduced motion on stops the loop, and switching it off restarts it', () => {
    const engine = mountEngine({ reducedMotion: false });
    expect(rafQueue).toHaveLength(1);
    rafQueue = [];
    engine.setReducedMotion(true);
    expect(rafQueue).toHaveLength(0);
    engine.setReducedMotion(false);
    expect(rafQueue).toHaveLength(1);
  });

  it('repaints the held world once when something changes, not once per change', () => {
    const paint = paintSpy();
    const engine = mountEngine({ reducedMotion: true });
    expect(paint).toHaveBeenCalledTimes(1);
    engine.setTod(0.5);
    engine.setTod(0.6);
    flushFrames();
    expect(paint).toHaveBeenCalledTimes(2);
  });

  it('a still repaint never steps the world clock, even after a frame at a later time', () => {
    const engine = mountEngine({ reducedMotion: false });
    // One real frame five seconds in: the world's clock is now at 5000ms.
    const pending = rafQueue;
    rafQueue = [];
    pending.forEach((cb) => cb(5000));

    const world = engine.state.world as unknown as { update: (dt: number) => void };
    const update = vi.spyOn(world, 'update');
    engine.setReducedMotion(true);
    expect(update).toHaveBeenCalled();
    const steps = update.mock.calls.map((call) => call[0]);
    // Without the fix the step is (0 - 5000) / 1000 = -5: time running backwards.
    expect(steps.every((dt) => dt === 0)).toBe(true);
  });

  it('does not repaint the held world when nothing changed', () => {
    const paint = paintSpy();
    mountEngine({ reducedMotion: true });
    flushFrames();
    expect(paint).toHaveBeenCalledTimes(1);
  });

  it('keeps the companion off under reduced motion, even on the full profile', () => {
    const engine = mountEngine({ reducedMotion: true, profile: 'full' });
    expect(companionOn(engine)).toBe(false);
  });

  it('shows the companion on the full profile without reduced motion', () => {
    const engine = mountEngine({ reducedMotion: false, profile: 'full' });
    expect(companionOn(engine)).toBe(true);
  });

  it('turning reduced motion off brings the companion back on the full profile', () => {
    const engine = mountEngine({ reducedMotion: true, profile: 'full' });
    engine.setReducedMotion(false);
    expect(companionOn(engine)).toBe(true);
  });
});

describe('AmbienceEngine companion over controls', () => {
  it('steps aside while it would sit on a control, and returns off one', () => {
    const engine = mountEngine({ reducedMotion: false, profile: 'full' });
    const button = document.createElement('button');
    document.body.appendChild(button);
    const elementFromPoint = vi.fn(() => button);
    Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: elementFromPoint });

    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 100, clientY: 100 }));
    // The companion trails 26px down and to the right of the pointer.
    expect(elementFromPoint).toHaveBeenCalledWith(126, 126);
    expect(companionOn(engine)).toBe(false);

    const plain = document.createElement('div');
    document.body.appendChild(plain);
    pointAt(plain);
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 100, clientY: 100 }));
    expect(companionOn(engine)).toBe(true);
    button.remove();
    plain.remove();
  });

  it('stays hidden over a control under reduced motion', () => {
    const engine = mountEngine({ reducedMotion: true, profile: 'full' });
    pointAt(document.createElement('div'));
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 100, clientY: 100 }));
    expect(companionOn(engine)).toBe(false);
  });
});

describe('coversContent', () => {
  it('is true for buttons, links, fields and anything inside them', () => {
    const button = document.createElement('button');
    const label = document.createElement('span');
    button.appendChild(label);
    const link = document.createElement('a');
    link.href = '#x';
    const input = document.createElement('input');
    const editable = document.createElement('div');
    editable.setAttribute('contenteditable', 'true');
    const tab = document.createElement('div');
    tab.setAttribute('role', 'tab');
    [button, link, input, editable, tab].forEach((el) => document.body.appendChild(el));
    expect(coversContent(button)).toBe(true);
    expect(coversContent(label)).toBe(true);
    expect(coversContent(link)).toBe(true);
    expect(coversContent(input)).toBe(true);
    expect(coversContent(editable)).toBe(true);
    expect(coversContent(tab)).toBe(true);
    [button, link, input, editable, tab].forEach((el) => el.remove());
  });

  it('is true for readable text that is not a control (the Import Instances label)', () => {
    const heading = document.createElement('h2');
    heading.textContent = 'Import Instances';
    const caption = document.createElement('span');
    caption.textContent = 'Create Instance';
    document.body.append(heading, caption);
    expect(coversContent(heading)).toBe(true);
    expect(coversContent(caption)).toBe(true);
    heading.remove();
    caption.remove();
  });

  it('is false for empty or whitespace-only text, plain containers, the canvas and nothing at all', () => {
    const blank = document.createElement('div');
    blank.textContent = '   \n\t ';
    const container = document.createElement('div');
    container.appendChild(document.createElement('span'));
    expect(coversContent(blank)).toBe(false);
    expect(coversContent(container)).toBe(false);
    expect(coversContent(document.createElement('div'))).toBe(false);
    expect(coversContent(document.createElement('canvas'))).toBe(false);
    expect(coversContent(null)).toBe(false);
  });
});

describe('buddyFootprint', () => {
  it('covers the whole companion body, not just its top-left corner', () => {
    const points = buddyFootprint(100, 200);
    // The far corner is 26 px right and 22 px down: a check at (100, 200) alone missed it.
    expect(points).toContainEqual([126, 222]);
    expect(points).toContainEqual([126, 200]);
    expect(points).toContainEqual([100, 222]);
    expect(points).toHaveLength(5);
  });

  it('stays on the origin for a zero position (no NaN or negative drift)', () => {
    expect(buddyFootprint(0, 0)).toContainEqual([0, 0]);
    expect(buddyFootprint(0, 0).every(([x, y]) => Number.isFinite(x) && Number.isFinite(y))).toBe(true);
  });
});
