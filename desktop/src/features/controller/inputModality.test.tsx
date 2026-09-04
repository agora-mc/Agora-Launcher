import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { GamepadIntent } from '../../lib/useGamepad';
import { ControllerProvider } from './ControllerProvider';
import { handleDirectInput, INPUT_MODALITY_ATTRIBUTE } from './inputModality';

const harness = vi.hoisted(() => ({
  onIntent: undefined as ((intent: GamepadIntent) => void) | undefined,
}));

vi.mock('../../lib/useGamepad', () => ({
  useGamepad: (options: { onIntent?: (intent: GamepadIntent) => void }) => {
    harness.onIntent = options.onIntent;
    return { connected: true, gamepadCount: 1 };
  },
}));

beforeEach(() => {
  harness.onIntent = undefined;
  document.documentElement.removeAttribute(INPUT_MODALITY_ATTRIBUTE);
});

afterEach(() => {
  document.documentElement.removeAttribute(INPUT_MODALITY_ATTRIBUTE);
});

const modality = () => document.documentElement.getAttribute(INPUT_MODALITY_ATTRIBUTE);

function pressSouth() {
  act(() => {
    harness.onIntent?.({ type: 'button', button: 'south' });
  });
}

/**
 * jsdom follows the spec in making `isTrusted` an unforgeable own property, so
 * no dispatched event can stand in for a real one. The handler is called
 * directly instead, which is what `watchForDirectInput` binds to the window.
 */
function useDirectInput() {
  act(() => {
    handleDirectInput({ isTrusted: true });
  });
}

describe('input modality', () => {
  it('is unset before any input', () => {
    render(<ControllerProvider><div /></ControllerProvider>);

    expect(modality()).toBeNull();
  });

  it('marks gamepad modality on the first controller intent', () => {
    render(<ControllerProvider><div /></ControllerProvider>);

    pressSouth();

    expect(modality()).toBe('gamepad');
  });

  it('marks gamepad modality even when no layer claims the intent', () => {
    render(<ControllerProvider><div /></ControllerProvider>);

    act(() => {
      harness.onIntent?.({ type: 'direction', direction: 'down' });
    });

    expect(modality()).toBe('gamepad');
  });

  it('clears when the user reaches for a mouse or keyboard', () => {
    render(<ControllerProvider><div /></ControllerProvider>);
    pressSouth();

    useDirectInput();

    expect(modality()).toBeNull();
  });

  it('ignores an untrusted event reaching the same handler', () => {
    render(<ControllerProvider><div /></ControllerProvider>);
    pressSouth();

    act(() => handleDirectInput({ isTrusted: false }));

    expect(modality()).toBe('gamepad');
  });

  /**
   * Widget adapters are expected to synthesise the arrow keys that Radix
   * composites already understand. That is still the controller talking, so it
   * must not knock the app out of gamepad modality mid-navigation.
   */
  it('ignores synthesised keyboard events', () => {
    render(<ControllerProvider><div /></ControllerProvider>);
    pressSouth();

    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    });

    expect(modality()).toBe('gamepad');
  });

  it('returns to gamepad modality after the controller is used again', () => {
    render(<ControllerProvider><div /></ControllerProvider>);
    pressSouth();
    useDirectInput();

    pressSouth();

    expect(modality()).toBe('gamepad');
  });
});
