import { useRef, useState, type ReactNode } from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
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

function press(button: 'south' | 'east') {
  act(() => {
    harness.onIntent?.({ type: 'button', button });
  });
}

function Root({ children }: { children: ReactNode }) {
  const rootRef = useRef<HTMLDivElement>(null);
  useControllerLayer({ rootRef, priority: CONTROLLER_LAYER_ROOT });
  return <div ref={rootRef}>{children}</div>;
}

function ControlledField({ initial = '' }: { initial?: string }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <input
        aria-label="name"
        value={value}
        onChange={(event) => setValue(event.target.value)}
      />
      <output role="status">{value}</output>
    </>
  );
}

function renderKeyboard(initial = '') {
  return render(
    <ControllerProvider>
      <Root>
        <ControlledField initial={initial} />
      </Root>
    </ControllerProvider>,
  );
}

describe('OnScreenKeyboard', () => {
  it('opens from an editable field without clicking it and edits the stored field after focus moves', () => {
    const clicked = vi.fn();
    render(
      <ControllerProvider>
        <Root>
          <input aria-label="name" onClick={clicked} defaultValue="ac" />
        </Root>
      </ControllerProvider>,
    );
    const field = screen.getByLabelText('name') as HTMLInputElement;
    field.focus();
    field.setSelectionRange(1, 1);

    press('south');

    expect(screen.getByRole('dialog', { name: 'On-screen keyboard' })).toBeInTheDocument();
    expect(clicked).not.toHaveBeenCalled();
    const bKey = screen.getByRole('button', { name: 'Type b' });
    bKey.focus();
    fireEvent.click(bKey);

    expect(field.value).toBe('abc');
    expect(document.activeElement).toBe(bKey);
  });

  it('supports uppercase and symbols, and physical typing while the key buttons own focus', () => {
    renderKeyboard();
    const field = screen.getByLabelText('name') as HTMLInputElement;
    field.focus();
    press('south');

    fireEvent.click(screen.getByRole('button', { name: 'Shift' }));
    fireEvent.click(screen.getByRole('button', { name: 'Type A' }));
    fireEvent.click(screen.getByRole('button', { name: 'Show numbers and symbols' }));
    fireEvent.click(screen.getByRole('button', { name: 'Type 1' }));
    fireEvent.keyDown(window, { key: 'z' });

    expect(screen.getByRole('status')).toHaveTextContent('A1z');
  });

  it('edits, cancels, and returns focus to the field', () => {
    renderKeyboard('abc');
    const field = screen.getByLabelText('name') as HTMLInputElement;
    field.focus();
    field.setSelectionRange(3, 3);
    press('south');

    fireEvent.click(screen.getByRole('button', { name: 'Backspace' }));
    expect(screen.getByRole('status')).toHaveTextContent('ab');

    press('east');

    expect(screen.queryByRole('dialog', { name: 'On-screen keyboard' })).not.toBeInTheDocument();
    expect(document.activeElement).toBe(field);
  });

  it('closes when the field node is replaced', async () => {
    const { rerender } = renderKeyboard('old');
    const field = screen.getByLabelText('name');
    field.focus();
    press('south');

    rerender(
      <ControllerProvider>
        <Root>
          <input aria-label="replacement" defaultValue="new" />
        </Root>
      </ControllerProvider>,
    );

    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: 'On-screen keyboard' })).not.toBeInTheDocument();
    });
  });
});
