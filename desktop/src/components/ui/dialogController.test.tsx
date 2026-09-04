/**
 * The bug this guards against, found by actually testing with a controller:
 * every dialog in the app was undriveable, and — worse — the page *behind* it
 * stayed driveable. The stick moved a highlight the user could not see, under a
 * dialog they could not answer.
 *
 * The fix lives in the shared `DialogContent` rather than in each dialog,
 * because "this dialog forgot to claim controller input" is not a failure any
 * individual author would notice.
 */
import { useRef, useState } from 'react';
import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GamepadIntent } from '@/lib/useGamepad';
import { ControllerProvider } from '@/features/controller/ControllerProvider';
import { CONTROLLER_LAYER_ROOT, useControllerLayer } from '@/features/controller/useControllerLayer';
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';

const harness = vi.hoisted(() => ({
  onIntent: undefined as ((intent: GamepadIntent) => void) | undefined,
}));

vi.mock('@/lib/useGamepad', () => ({
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

/** Stands in for the application shell underneath a dialog. */
function Background({ onIntent }: { onIntent: (intent: unknown) => void }) {
  const rootRef = useRef<HTMLDivElement>(null);
  useControllerLayer({
    rootRef,
    priority: CONTROLLER_LAYER_ROOT,
    onIntent: (intent) => { onIntent(intent); return true; },
  });
  return <div ref={rootRef}><button type="button">behind</button></div>;
}

function Harness({ background }: { background: (intent: unknown) => void }) {
  const [open, setOpen] = useState(true);
  return (
    <ControllerProvider>
      <Background onIntent={background} />
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogTitle>A question</DialogTitle>
          <button type="button">answer</button>
        </DialogContent>
      </Dialog>
    </ControllerProvider>
  );
}

describe('dialogs and controller input', () => {
  it('keeps input off the page behind it while open', () => {
    const background = vi.fn();
    render(<Harness background={background} />);

    press('south');
    press('north');
    move();

    expect(background).not.toHaveBeenCalled();
  });

  it('activates the focused control inside the dialog', () => {
    render(<Harness background={vi.fn()} />);

    const answer = screen.getByText('answer');
    const clicked = vi.fn();
    answer.addEventListener('click', clicked);
    answer.focus();

    press('south');

    expect(clicked).toHaveBeenCalledTimes(1);
  });

  it('closes on cancel, and gives the page back afterwards', () => {
    const background = vi.fn();
    render(<Harness background={background} />);

    press('east');

    expect(screen.queryByText('answer')).not.toBeInTheDocument();

    press('south');
    expect(background).toHaveBeenCalled();
  });
});

function move() {
  act(() => {
    harness.onIntent?.({ type: 'direction', direction: 'down' });
  });
}
