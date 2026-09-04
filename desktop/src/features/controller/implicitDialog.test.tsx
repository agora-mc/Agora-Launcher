/**
 * Hand-rolled dialogs claiming controller input by attribute.
 *
 * The High Interaction surfaces build their own modals — a scrim div with a
 * dialog inside — and they sit behind an enforced import boundary that forbids
 * them from importing app-level modules. Without a way in, the "Ready to play"
 * overlay was undriveable and the page behind it kept control, which is the
 * same failure the shared Radix dialog had.
 */
import { useRef, useState } from 'react';
import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GamepadIntent } from '../../lib/useGamepad';
import { ControllerProvider } from './ControllerProvider';
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

/** Mirrors the real scrims: always mounted, hidden with a class, dismissed by
 *  a click that lands on the scrim itself rather than the dialog body. */
function Scrim({ background }: { background: (intent: unknown) => void }) {
  const [open, setOpen] = useState(true);
  const rootRef = useRef<HTMLDivElement>(null);
  useControllerLayer({
    rootRef,
    priority: CONTROLLER_LAYER_ROOT,
    onIntent: (intent) => { background(intent); return true; },
  });
  return (
    <div ref={rootRef}>
      <button type="button">behind</button>
      <div
        className={open ? 'scrim show' : 'scrim'}
        data-controller-dialog={open ? '' : undefined}
        onClick={(event) => { if (event.target === event.currentTarget) setOpen(false); }}
      >
        <div role="dialog" aria-modal="true" aria-label="Ready to play">
          <button type="button">Launch Minecraft</button>
        </div>
      </div>
    </div>
  );
}

describe('attribute-declared dialogs', () => {
  it('takes controller input away from the page behind it', () => {
    const background = vi.fn();
    render(<ControllerProvider><Scrim background={background} /></ControllerProvider>);

    press('south');

    expect(background).not.toHaveBeenCalled();
  });

  it('activates a control inside the overlay', () => {
    render(<ControllerProvider><Scrim background={vi.fn()} /></ControllerProvider>);

    const launch = screen.getByText('Launch Minecraft');
    const clicked = vi.fn();
    launch.addEventListener('click', clicked);
    launch.focus();

    press('south');

    expect(clicked).toHaveBeenCalledTimes(1);
  });

  it('dismisses on cancel using the scrim its own click handler expects', () => {
    render(<ControllerProvider><Scrim background={vi.fn()} /></ControllerProvider>);

    press('east');

    expect(screen.queryByText('Launch Minecraft')).toBeInTheDocument();
    expect(document.querySelector('[data-controller-dialog]')).toBeNull();
  });

  it('gives the page back once the overlay is closed', () => {
    const background = vi.fn();
    render(<ControllerProvider><Scrim background={background} /></ControllerProvider>);

    press('east');
    press('south');

    expect(background).toHaveBeenCalled();
  });

  it('ignores a scrim that is mounted but not open', () => {
    const background = vi.fn();
    function Closed() {
      const rootRef = useRef<HTMLDivElement>(null);
      useControllerLayer({
        rootRef,
        priority: CONTROLLER_LAYER_ROOT,
        onIntent: (intent) => { background(intent); return true; },
      });
      return (
        <div ref={rootRef}>
          <button type="button">behind</button>
          {/* Mounted, hidden by class, and deliberately carrying no attribute. */}
          <div className="scrim" onClick={() => undefined} />
        </div>
      );
    }
    render(<ControllerProvider><Closed /></ControllerProvider>);

    press('south');

    expect(background).toHaveBeenCalled();
  });
});
