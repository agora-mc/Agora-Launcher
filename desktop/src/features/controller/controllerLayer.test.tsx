/**
 * Contract for the controller input-ownership stack.
 *
 * These tests were written before the implementation and define the API rather
 * than describe it. The central rule is that exactly one layer owns input at a
 * time and every intent has somewhere to go: the failure mode this guards
 * against is not a crash, it is a controller that silently stops responding on
 * one screen because an intent fell into a gap.
 */
import { useRef } from 'react';
import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GamepadIntent } from '../../lib/useGamepad';
import { ControllerProvider } from './ControllerProvider';
import { useControllerLayer } from './useControllerLayer';

const harness = vi.hoisted(() => ({
  connected: true,
  onIntent: undefined as ((intent: GamepadIntent) => void) | undefined,
  onConnectionChange: undefined as ((connected: boolean) => void) | undefined,
}));

vi.mock('../../lib/useGamepad', () => ({
  useGamepad: (options: {
    onIntent?: (intent: GamepadIntent) => void;
    onConnectionChange?: (connected: boolean) => void;
  }) => {
    harness.onIntent = options.onIntent;
    harness.onConnectionChange = options.onConnectionChange;
    return { connected: harness.connected, gamepadCount: harness.connected ? 1 : 0 };
  },
}));

beforeEach(() => {
  harness.connected = true;
  harness.onIntent = undefined;
  harness.onConnectionChange = undefined;
});

/** Push a raw gamepad intent through the provider, as the sampler would. */
function send(intent: GamepadIntent) {
  act(() => {
    harness.onIntent?.(intent);
  });
}

const press = (button: string) => send({ type: 'button', button } as GamepadIntent);
const move = (direction: 'up' | 'down' | 'left' | 'right') =>
  send({ type: 'direction', direction });

/** A layer whose root holds `count` buttons, for default-navigation tests. */
function ButtonLayer({
  count,
  label,
  onIntent,
  onCancel,
  active = true,
  onClickIndex,
}: {
  count: number;
  label: string;
  onIntent?: (intent: unknown) => boolean | void;
  onCancel?: () => void;
  active?: boolean;
  onClickIndex?: (index: number) => void;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  useControllerLayer({ active, rootRef, onIntent, onCancel });
  return (
    <div ref={rootRef}>
      {Array.from({ length: count }, (_, index) => (
        <button key={index} type="button" onClick={() => onClickIndex?.(index)}>
          {`${label}-${index}`}
        </button>
      ))}
    </div>
  );
}

describe('controller layer stack', () => {
  it('does not throw when no layer is registered', () => {
    render(<ControllerProvider><div /></ControllerProvider>);

    expect(() => {
      move('up');
      move('down');
      move('left');
      move('right');
      press('south');
      press('east');
      press('west');
      press('north');
      press('start');
      press('l1');
      press('r1');
      send({ type: 'stick', direction: 'down' } as GamepadIntent);
    }).not.toThrow();
  });

  it('delivers intents only to the topmost layer', () => {
    const lower = vi.fn();
    const upper = vi.fn();
    render(
      <ControllerProvider>
        <ButtonLayer count={2} label="lower" onIntent={lower} />
        <ButtonLayer count={2} label="upper" onIntent={upper} />
      </ControllerProvider>,
    );

    press('south');

    expect(upper).toHaveBeenCalled();
    expect(lower).not.toHaveBeenCalled();
  });

  it('returns ownership to the layer beneath when the top one unmounts', () => {
    const lower = vi.fn();
    const upper = vi.fn();
    const { rerender } = render(
      <ControllerProvider>
        <ButtonLayer count={2} label="lower" onIntent={lower} />
        <ButtonLayer count={2} label="upper" onIntent={upper} />
      </ControllerProvider>,
    );

    rerender(
      <ControllerProvider>
        <ButtonLayer count={2} label="lower" onIntent={lower} />
      </ControllerProvider>,
    );
    press('south');

    expect(lower).toHaveBeenCalled();
    expect(upper).not.toHaveBeenCalled();
  });

  it('does not register an inactive layer', () => {
    const inactive = vi.fn();
    const active = vi.fn();
    render(
      <ControllerProvider>
        <ButtonLayer count={2} label="active" onIntent={active} />
        <ButtonLayer count={2} label="inactive" onIntent={inactive} active={false} />
      </ControllerProvider>,
    );

    press('south');

    expect(active).toHaveBeenCalled();
    expect(inactive).not.toHaveBeenCalled();
  });

  it('survives a layer whose root has no focusable children', () => {
    render(
      <ControllerProvider>
        <ButtonLayer count={0} label="empty" />
      </ControllerProvider>,
    );

    expect(() => {
      move('down');
      move('down');
      press('south');
    }).not.toThrow();
  });

  it('survives a layer whose root ref never attaches', () => {
    function DetachedLayer() {
      const rootRef = useRef<HTMLDivElement>(null);
      useControllerLayer({ rootRef });
      return null;
    }
    render(<ControllerProvider><DetachedLayer /></ControllerProvider>);

    expect(() => {
      move('down');
      press('south');
    }).not.toThrow();
  });
});

describe('default navigation', () => {
  it('moves focus through the layer root and wraps at the end', () => {
    render(
      <ControllerProvider>
        <ButtonLayer count={3} label="btn" />
      </ControllerProvider>,
    );

    move('down');
    expect(document.activeElement).toBe(screen.getByText('btn-0'));
    move('down');
    expect(document.activeElement).toBe(screen.getByText('btn-1'));
    move('down');
    expect(document.activeElement).toBe(screen.getByText('btn-2'));
    move('down');
    expect(document.activeElement).toBe(screen.getByText('btn-0'));
  });

  it('walks backwards for up and left', () => {
    render(
      <ControllerProvider>
        <ButtonLayer count={3} label="btn" />
      </ControllerProvider>,
    );

    move('up');
    move('up');
    expect(document.activeElement).toBe(screen.getByText('btn-1'));
    move('left');
    expect(document.activeElement).toBe(screen.getByText('btn-0'));
  });

  it('activates the focused control on accept', () => {
    const onClickIndex = vi.fn();
    render(
      <ControllerProvider>
        <ButtonLayer count={3} label="btn" onClickIndex={onClickIndex} />
      </ControllerProvider>,
    );

    move('down');
    move('down');
    press('south');

    expect(onClickIndex).toHaveBeenCalledWith(1);
  });

  it('does not activate anything when nothing inside the layer is focused', () => {
    const onClickIndex = vi.fn();
    render(
      <ControllerProvider>
        <ButtonLayer count={3} label="btn" onClickIndex={onClickIndex} />
      </ControllerProvider>,
    );

    press('south');

    expect(onClickIndex).not.toHaveBeenCalled();
  });

  it('calls onCancel for cancel', () => {
    const onCancel = vi.fn();
    render(
      <ControllerProvider>
        <ButtonLayer count={2} label="btn" onCancel={onCancel} />
      </ControllerProvider>,
    );

    press('east');

    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('suppresses default navigation when onIntent returns true', () => {
    render(
      <ControllerProvider>
        <ButtonLayer count={3} label="btn" onIntent={() => true} />
      </ControllerProvider>,
    );

    move('down');

    expect(document.activeElement).not.toBe(screen.getByText('btn-0'));
  });

  it('falls through to default navigation when onIntent returns nothing', () => {
    const onIntent = vi.fn();
    render(
      <ControllerProvider>
        <ButtonLayer count={3} label="btn" onIntent={onIntent} />
      </ControllerProvider>,
    );

    move('down');

    expect(onIntent).toHaveBeenCalled();
    expect(document.activeElement).toBe(screen.getByText('btn-0'));
  });

  it('skips disabled controls', () => {
    function DisabledLayer() {
      const rootRef = useRef<HTMLDivElement>(null);
      useControllerLayer({ rootRef });
      return (
        <div ref={rootRef}>
          <button type="button" disabled>skipped</button>
          <button type="button">reachable</button>
        </div>
      );
    }
    render(<ControllerProvider><DisabledLayer /></ControllerProvider>);

    move('down');

    expect(document.activeElement).toBe(screen.getByText('reachable'));
  });
});

describe('semantic mapping', () => {
  it('maps face and shoulder buttons to semantic intents, never to letters', () => {
    const seen: string[] = [];
    render(
      <ControllerProvider>
        <ButtonLayer
          count={1}
          label="btn"
          onIntent={(intent) => {
            seen.push((intent as { type: string }).type);
            return true;
          }}
        />
      </ControllerProvider>,
    );

    press('south');
    press('east');
    press('west');
    press('north');
    press('start');
    press('l1');
    press('r1');

    expect(seen).toEqual([
      'accept',
      'cancel',
      'secondary',
      'context',
      'menu',
      'page',
      'page',
    ]);
  });
});
