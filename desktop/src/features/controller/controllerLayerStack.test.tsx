/**
 * Ordering and pass-through rules for the layer stack.
 *
 * Both exist because of a concrete bug rather than in the abstract. The closed
 * handheld shell wants exactly one button and nothing else; left opaque it sat
 * on top of the whole application — which is its normal state — and swallowed
 * every other intent before the shell beneath could act. And the application
 * shell itself must sit at the bottom, which registration order cannot deliver
 * because React runs child effects before parent effects.
 */
import { useRef } from 'react';
import { act, render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GamepadIntent } from '../../lib/useGamepad';
import { ControllerProvider } from './ControllerProvider';
import { cycleTab } from './ControllerRootBindings';
import { CONTROLLER_LAYER_ROOT, useControllerLayer } from './useControllerLayer';

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
});

function press(button: string) {
  act(() => {
    harness.onIntent?.({ type: 'button', button } as GamepadIntent);
  });
}

function Layer({
  onIntent,
  onCancel,
  priority,
  transparent,
}: {
  onIntent?: (intent: unknown) => boolean | void;
  onCancel?: () => void;
  priority?: number;
  transparent?: boolean;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  useControllerLayer({ rootRef, onIntent, onCancel, priority, transparent });
  return <div ref={rootRef} />;
}

describe('layer priority', () => {
  it('keeps a root layer beneath one that mounted earlier', () => {
    const root = vi.fn();
    const normal = vi.fn();
    render(
      <ControllerProvider>
        <Layer onIntent={normal} />
        <Layer onIntent={root} priority={CONTROLLER_LAYER_ROOT} />
      </ControllerProvider>,
    );

    press('south');

    expect(normal).toHaveBeenCalled();
    expect(root).not.toHaveBeenCalled();
  });

  it('reaches the root layer once nothing above claims the intent', () => {
    const root = vi.fn();
    render(
      <ControllerProvider>
        <Layer onIntent={() => undefined} transparent />
        <Layer onIntent={root} priority={CONTROLLER_LAYER_ROOT} />
      </ControllerProvider>,
    );

    press('south');

    expect(root).toHaveBeenCalled();
  });
});

describe('transparent layers', () => {
  it('passes an unclaimed intent to the layer below', () => {
    const below = vi.fn();
    const above = vi.fn(() => undefined);
    render(
      <ControllerProvider>
        <Layer onIntent={below} />
        <Layer onIntent={above} transparent />
      </ControllerProvider>,
    );

    press('south');

    expect(above).toHaveBeenCalled();
    expect(below).toHaveBeenCalled();
  });

  it('stops dispatch for an intent it does claim', () => {
    const below = vi.fn();
    render(
      <ControllerProvider>
        <Layer onIntent={below} />
        <Layer onIntent={(intent) => (intent as { type: string }).type === 'menu'} transparent />
      </ControllerProvider>,
    );

    press('start');
    expect(below).not.toHaveBeenCalled();

    press('south');
    expect(below).toHaveBeenCalled();
  });

  it('does not let an opaque layer leak unclaimed intents downwards', () => {
    const below = vi.fn();
    render(
      <ControllerProvider>
        <Layer onIntent={below} />
        <Layer onIntent={() => undefined} />
      </ControllerProvider>,
    );

    press('south');

    expect(below).not.toHaveBeenCalled();
  });

  it('does not let a transparent layer trigger the layer below twice', () => {
    const onCancel = vi.fn();
    render(
      <ControllerProvider>
        <Layer onCancel={onCancel} />
        <Layer transparent />
      </ControllerProvider>,
    );

    press('east');

    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});

describe('cycleTab', () => {
  const tabs = ['home', 'browse', 'instances'] as const;

  it('steps forward and back', () => {
    expect(cycleTab(tabs, 'home', 'next')).toBe('browse');
    expect(cycleTab(tabs, 'browse', 'prev')).toBe('home');
  });

  it('clamps at both ends rather than wrapping', () => {
    expect(cycleTab(tabs, 'instances', 'next')).toBe('instances');
    expect(cycleTab(tabs, 'home', 'prev')).toBe('home');
  });

  it('falls back to the first tab when the current one is not in the list', () => {
    expect(cycleTab(tabs, 'settings' as never, 'next')).toBe('home');
  });

  it('survives an empty tab list', () => {
    expect(cycleTab([] as readonly string[], 'home', 'next')).toBe('home');
  });
});
