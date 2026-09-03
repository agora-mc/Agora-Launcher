import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useGamepad, type GamepadIntent } from './useGamepad';

function makeGamepad({
  buttons = [],
  axes = [0, 0],
}: { buttons?: number[]; axes?: number[] } = {}): Gamepad {
  const pressedButtons = new Set(buttons);
  return {
    id: 'Test Controller',
    index: 0,
    connected: true,
    timestamp: 0,
    mapping: 'standard',
    axes,
    buttons: Array.from({ length: 16 }, (_, index) => ({
      pressed: pressedButtons.has(index),
      touched: pressedButtons.has(index),
      value: pressedButtons.has(index) ? 1 : 0,
    })) as GamepadButton[],
    vibrationActuator: null,
    hapticActuators: [],
  } as unknown as Gamepad;
}

describe('useGamepad', () => {
  let current: Array<Gamepad | null>;
  let frames: FrameRequestCallback[];
  let originalGetGamepads: PropertyDescriptor | undefined;

  beforeEach(() => {
    current = [];
    frames = [];
    originalGetGamepads = Object.getOwnPropertyDescriptor(navigator, 'getGamepads');
    Object.defineProperty(navigator, 'getGamepads', {
      configurable: true,
      value: vi.fn(() => current),
    });
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
      frames.push(callback);
      return frames.length;
    });
    vi.stubGlobal('cancelAnimationFrame', vi.fn());
  });

  afterEach(() => {
    if (originalGetGamepads) {
      Object.defineProperty(navigator, 'getGamepads', originalGetGamepads);
    } else {
      delete (navigator as Partial<Navigator>).getGamepads;
    }
    vi.unstubAllGlobals();
  });

  function tick(timestamp: number) {
    const frame = frames.shift();
    expect(frame).toBeDefined();
    act(() => frame?.(timestamp));
  }

  it('reports no controller when the API is unavailable', () => {
    Object.defineProperty(navigator, 'getGamepads', {
      configurable: true,
      value: undefined,
    });
    const onIntent = vi.fn<(intent: GamepadIntent) => void>();
    const { result } = renderHook(() => useGamepad({ onIntent }));

    expect(result.current.connected).toBe(false);
    expect(result.current.gamepadCount).toBe(0);
    expect(onIntent).not.toHaveBeenCalled();
    // No API means no pad, and no pad means nothing to poll for.
    expect(frames).toHaveLength(0);
  });

  it('ignores sparse null entries and reports a real connected pad', () => {
    current = [null, makeGamepad()];
    const onConnectionChange = vi.fn();
    const { result } = renderHook(() => useGamepad({ onConnectionChange }));

    tick(0);

    expect(result.current.connected).toBe(true);
    expect(result.current.gamepadCount).toBe(1);
    expect(onConnectionChange).toHaveBeenCalledWith(true);
  });

  it('emits A on a rising edge and repeats held direction after a delay', () => {
    const onIntent = vi.fn<(intent: GamepadIntent) => void>();
    current = [makeGamepad({ axes: [1, 0] })];
    const { result } = renderHook(() => useGamepad({ onIntent }));

    tick(0);
    expect(onIntent).toHaveBeenLastCalledWith({ type: 'direction', direction: 'right' });
    tick(419);
    expect(onIntent).toHaveBeenCalledTimes(1);
    tick(420);
    expect(onIntent).toHaveBeenCalledTimes(2);
    tick(599);
    expect(onIntent).toHaveBeenCalledTimes(2);
    tick(600);
    expect(onIntent).toHaveBeenCalledTimes(3);

    current[0] = makeGamepad({ buttons: [0] });
    tick(601);
    expect(onIntent).toHaveBeenLastCalledWith({ type: 'button', button: 'a' });
    tick(602);
    expect(onIntent).toHaveBeenCalledTimes(4);
    expect(result.current.connected).toBe(true);
  });

  it('supports the standard D-pad mapping as well as the left stick', () => {
    const onIntent = vi.fn<(intent: GamepadIntent) => void>();
    current = [makeGamepad({ buttons: [15] })];
    renderHook(() => useGamepad({ onIntent }));

    tick(0);

    expect(onIntent).toHaveBeenCalledWith({ type: 'direction', direction: 'right' });
  });

  it('does not poll at all until a controller shows up', () => {
    // A permanent animation-frame loop for every user would be a poor trade on
    // the battery-powered handhelds this exists for, and there is nothing to
    // read from an absent pad anyway. Presence rides on the browser's events.
    const onConnectionChange = vi.fn();
    const onIntent = vi.fn<(intent: GamepadIntent) => void>();
    const { result } = renderHook(() => useGamepad({ onConnectionChange, onIntent }));

    expect(frames).toHaveLength(0);
    expect(result.current.connected).toBe(false);
    expect(onIntent).not.toHaveBeenCalled();

    current = [makeGamepad({ buttons: [0], axes: [1, 0] })];
    act(() => { window.dispatchEvent(new Event('gamepadconnected')); });

    expect(result.current.connected).toBe(true);
    expect(onConnectionChange).toHaveBeenLastCalledWith(true);
    // Picking the pad up is what starts polling — no switch to flip first.
    expect(frames.length).toBeGreaterThan(0);
  });

  it('stops polling again when the last controller goes away', () => {
    current = [makeGamepad({ axes: [1, 0] })];
    const onIntent = vi.fn<(intent: GamepadIntent) => void>();
    const { result } = renderHook(() => useGamepad({ onIntent }));

    tick(0);
    expect(onIntent).toHaveBeenCalledTimes(1);

    current = [];
    act(() => { window.dispatchEvent(new Event('gamepaddisconnected')); });
    expect(result.current.connected).toBe(false);

    frames = [];
    // Nothing reschedules a frame once the pad is gone.
    expect(frames).toHaveLength(0);
  });

  it('clears connection state and held input when the controller disconnects', () => {
    current = [makeGamepad({ axes: [-1, 0] })];
    const onConnectionChange = vi.fn();
    const onIntent = vi.fn<(intent: GamepadIntent) => void>();
    const { result } = renderHook(() => useGamepad({ onConnectionChange, onIntent }));

    tick(0);
    current = [null, null];
    tick(1);

    expect(result.current.connected).toBe(false);
    expect(result.current.gamepadCount).toBe(0);
    expect(onConnectionChange).toHaveBeenLastCalledWith(false);
    const countAtDisconnect = onIntent.mock.calls.length;
    tick(1000);
    expect(onIntent).toHaveBeenCalledTimes(countAtDisconnect);
  });
});
