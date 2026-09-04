/**
 * `<input type="color">` opens an operating-system colour dialog that the
 * Gamepad API cannot see or steer, which left the seven theme colour settings
 * simply unadjustable on a handheld. This is the in-app replacement.
 */
import { useState } from 'react';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GamepadIntent } from '../../lib/useGamepad';
import { ControllerProvider } from './ControllerProvider';
import { ColorOverlay } from './ColorOverlay';

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

/** Controlled, because the whole point is that React's onChange must fire. */
function Harness({ onClose = vi.fn(), initial = '#2563eb' }: { onClose?: () => void; initial?: string }) {
  const [value, setValue] = useState(initial);
  const [node, setNode] = useState<HTMLInputElement | null>(null);
  return (
    <ControllerProvider>
      <input
        type="color"
        aria-label="Accent colour"
        ref={setNode}
        value={value}
        onChange={(event) => setValue(event.target.value)}
      />
      <output>{value}</output>
      {node && <ColorOverlay input={node} onClose={onClose} />}
    </ControllerProvider>
  );
}

const chosen = () => screen.getByRole('status', { hidden: true }).textContent;
const overlay = () => within(screen.getByRole('dialog'));

describe('ColorOverlay', () => {
  it('picks a preset and writes it through React', () => {
    render(<Harness />);

    act(() => { overlay().getByLabelText('#16a34a').click(); });

    expect(chosen()).toBe('#16a34a');
  });

  it('adjusts a single channel with a slider', () => {
    render(<Harness initial="#000000" />);

    fireEvent.change(overlay().getByLabelText('Red'), { target: { value: '255' } });

    expect(chosen()).toBe('#ff0000');
  });

  it('restores the original colour on cancel', () => {
    const onClose = vi.fn();
    render(<Harness onClose={onClose} initial="#2563eb" />);

    act(() => { overlay().getByLabelText('#16a34a').click(); });
    expect(chosen()).toBe('#16a34a');

    press('east');

    expect(chosen()).toBe('#2563eb');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('keeps the previewed colour when confirmed', () => {
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);

    act(() => { overlay().getByLabelText('#059669').click(); });
    act(() => { overlay().getByText('Done').click(); });

    expect(chosen()).toBe('#059669');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('owns controller input while open', () => {
    render(<Harness />);

    const swatch = overlay().getByLabelText('#7c3aed');
    swatch.focus();
    press('south');

    expect(chosen()).toBe('#7c3aed');
  });

  it('reads a malformed value as black rather than throwing', () => {
    expect(() => render(<Harness initial="not-a-colour" />)).not.toThrow();
  });
});
