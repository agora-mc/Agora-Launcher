import { useState } from 'react';
import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { GAMEPAD_MODALITY, INPUT_MODALITY_ATTRIBUTE } from './inputModality';
import { focusMemoryKey, useFocusMemory } from './useFocusMemory';

afterEach(() => {
  document.documentElement.removeAttribute(INPUT_MODALITY_ATTRIBUTE);
});

function useGamepadModality(on: boolean) {
  if (on) document.documentElement.setAttribute(INPUT_MODALITY_ATTRIBUTE, GAMEPAD_MODALITY);
  else document.documentElement.removeAttribute(INPUT_MODALITY_ATTRIBUTE);
}

/** Two "pages", both permanently mounted, mirroring how App keeps Browse and
 *  InstanceEditor alive but hidden behind a detail destination. */
function Harness({ initial = 'a' }: { initial?: string }) {
  const [key, setKey] = useState(initial);
  useFocusMemory(key);
  return (
    <div>
      <button type="button" onClick={() => setKey('a')}>go-a</button>
      <button type="button" onClick={() => setKey('b')}>go-b</button>
      <div hidden={key !== 'a'}>
        <button type="button">a-one</button>
        <button type="button">a-two</button>
      </div>
      <div hidden={key !== 'b'}>
        <button type="button">b-one</button>
      </div>
    </div>
  );
}

describe('useFocusMemory', () => {
  it('restores the remembered element when returning on a controller', async () => {
    useGamepadModality(true);
    render(<Harness />);

    screen.getByText('a-two').focus();
    act(() => screen.getByText('go-b').click());
    screen.getByText('b-one').focus();
    act(() => screen.getByText('go-a').click());

    await waitFor(() => expect(document.activeElement).toBe(screen.getByText('a-two')));
  });

  it('leaves focus alone for a mouse or keyboard user', async () => {
    useGamepadModality(false);
    render(<Harness />);

    screen.getByText('a-two').focus();
    act(() => screen.getByText('go-b').click());
    const afterLeaving = document.activeElement;
    act(() => screen.getByText('go-a').click());

    // Nothing is restored; focus stays wherever the click left it.
    await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
    expect(document.activeElement).toBe(afterLeaving);
  });

  it('does not throw when the remembered element has gone away', async () => {
    useGamepadModality(true);
    function Vanishing() {
      const [key, setKey] = useState('a');
      useFocusMemory(key);
      return (
        <div>
          <button type="button" onClick={() => setKey(key === 'a' ? 'b' : 'a')}>swap</button>
          {key === 'a' && <button type="button">transient</button>}
        </div>
      );
    }
    render(<Vanishing />);

    screen.getByText('transient').focus();
    act(() => screen.getByText('swap').click());

    await expect(
      (async () => {
        act(() => screen.getByText('swap').click());
        await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
      })(),
    ).resolves.not.toThrow();
  });
});

describe('focusMemoryKey', () => {
  it('distinguishes every destination shape', () => {
    const keys = [
      focusMemoryKey({ type: 'tab', tab: 'browse' }),
      focusMemoryKey({ type: 'tab', tab: 'settings' }),
      focusMemoryKey({ type: 'mod-detail', itemId: 'sodium' }),
      focusMemoryKey({ type: 'instance-detail', instanceId: 'inst-1' }),
    ];

    expect(new Set(keys).size).toBe(keys.length);
  });

  it('treats the same destination as the same key', () => {
    expect(focusMemoryKey({ type: 'mod-detail', itemId: 'sodium' }))
      .toBe(focusMemoryKey({ type: 'mod-detail', itemId: 'sodium' }));
  });
});
