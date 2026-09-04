/**
 * Dropdowns were the loudest complaint from real controller testing, for two
 * reasons. The system popup a `<select>` opens is invisible to the Gamepad API,
 * and the earlier workaround — cycling options with up and down — turned every
 * dropdown in a column of settings into a trap vertical movement could not get
 * past. This overlay replaces both.
 */
import { useState } from 'react';
import { act, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GamepadIntent } from '../../lib/useGamepad';
import { ControllerProvider } from './ControllerProvider';
import { SelectOverlay } from './SelectOverlay';

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

/** A controlled select plus the overlay, mirroring how the provider mounts it. */
function Harness({ onClose = vi.fn() }: { onClose?: () => void }) {
  const [value, setValue] = useState('b');
  const [node, setNode] = useState<HTMLSelectElement | null>(null);
  return (
    <ControllerProvider>
      <select
        aria-label="Loader"
        ref={setNode}
        value={value}
        onChange={(event) => setValue(event.target.value)}
      >
        <option value="a">Fabric</option>
        <option value="b">Quilt</option>
        <option value="c">NeoForge</option>
      </select>
      <output>{value}</output>
      {node && <SelectOverlay select={node} onClose={onClose} />}
    </ControllerProvider>
  );
}

const chosen = () => screen.getByRole('status', { hidden: true }).textContent;
/** The native <option> elements carry the same labels, so queries must be
 *  scoped to the overlay or they match twice. */
const overlay = () => within(screen.getByRole('dialog'));

describe('SelectOverlay', () => {
  it('offers every enabled option', () => {
    render(<Harness />);

    expect(overlay().getByText('Fabric')).toBeInTheDocument();
    expect(overlay().getByText('Quilt')).toBeInTheDocument();
    expect(overlay().getByText('NeoForge')).toBeInTheDocument();
  });

  it('marks the option that is currently selected', () => {
    render(<Harness />);

    expect(overlay().getByText('Quilt').closest('button')).toHaveAttribute('aria-pressed', 'true');
    expect(overlay().getByText('Fabric').closest('button')).toHaveAttribute('aria-pressed', 'false');
  });

  it('commits a choice through React and closes', () => {
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);

    act(() => { overlay().getByText('NeoForge').closest('button')?.click(); });

    expect(chosen()).toBe('c');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('leaves the value alone on cancel', () => {
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);

    press('east');

    expect(chosen()).toBe('b');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('owns controller input while it is open', () => {
    render(<Harness />);

    // Accept lands on a focused option inside the overlay rather than falling
    // through to whatever is behind it.
    const option = overlay().getByText('Fabric').closest('button') as HTMLButtonElement;
    option.focus();
    press('south');

    expect(chosen()).toBe('a');
  });

  it('renders an empty select without throwing', () => {
    function Empty() {
      const [node, setNode] = useState<HTMLSelectElement | null>(null);
      return (
        <ControllerProvider>
          <select aria-label="Nothing" ref={setNode} />
          {node && <SelectOverlay select={node} onClose={vi.fn()} />}
        </ControllerProvider>
      );
    }

    expect(() => render(<Empty />)).not.toThrow();
    expect(screen.getByText('Nothing to choose from.')).toBeInTheDocument();
  });
});
