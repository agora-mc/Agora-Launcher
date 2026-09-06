/**
 * The regression these cover: `goBack()` delegates to `window.history.back()`,
 * whose `popstate` came back through the same handler that `push` used and
 * appended to the internal stack again. Navigating *backwards* therefore grew
 * the history, and `canGoBack` never returned to false. It was mostly invisible
 * while only a UI affordance depended on it; binding a controller's Cancel to
 * it would have made it obvious.
 */
import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { useDestination } from './useDestination';

/**
 * jsdom implements pushState but never fires `popstate` for `history.back()`,
 * so the round trip has to be driven explicitly. Entries are tracked here the
 * way a browser would, and `back()` dispatches the event with the state it
 * lands on.
 */
function installHistory() {
  const entries: unknown[] = [null];
  let index = 0;

  const history = {
    get state() { return entries[index]; },
    pushState(state: unknown) {
      entries.splice(index + 1);
      entries.push(state);
      index = entries.length - 1;
    },
    replaceState(state: unknown) { entries[index] = state; },
    back() {
      if (index === 0) return;
      index -= 1;
      window.dispatchEvent(new PopStateEvent('popstate', { state: entries[index] }));
    },
  };

  Object.defineProperty(window, 'history', { value: history, configurable: true, writable: true });
  return { depth: () => index };
}

let harness: ReturnType<typeof installHistory>;

beforeEach(() => {
  harness = installHistory();
});

describe('useDestination', () => {
  it('starts at home with nothing behind it', () => {
    const { result } = renderHook(() => useDestination());

    expect(result.current.destination).toEqual({ type: 'tab', tab: 'home' });
    expect(result.current.canGoBack).toBe(false);
  });

  it('navigates forward and reports that back is available', () => {
    const { result } = renderHook(() => useDestination());

    act(() => result.current.navigateToTab('browse'));

    expect(result.current.destination).toEqual({ type: 'tab', tab: 'browse' });
    expect(result.current.canGoBack).toBe(true);
  });

  it('returns canGoBack to false after stepping back to the root', () => {
    const { result } = renderHook(() => useDestination());

    act(() => result.current.navigateToTab('browse'));
    act(() => result.current.goBack());

    expect(result.current.destination).toEqual({ type: 'tab', tab: 'home' });
    expect(result.current.canGoBack).toBe(false);
  });

  it('does not grow the history when navigating backwards', () => {
    const { result } = renderHook(() => useDestination());

    act(() => result.current.navigateToTab('browse'));
    act(() => result.current.navigateToModDetail('sodium'));
    expect(harness.depth()).toBe(2);

    act(() => result.current.goBack());
    expect(harness.depth()).toBe(1);
    expect(result.current.canGoBack).toBe(true);

    act(() => result.current.goBack());
    expect(harness.depth()).toBe(0);
    expect(result.current.canGoBack).toBe(false);
  });

  it('survives repeated back presses at the root without leaving the app', () => {
    const { result } = renderHook(() => useDestination());

    act(() => result.current.navigateToTab('settings'));
    act(() => result.current.goBack());
    act(() => result.current.goBack());
    act(() => result.current.goBack());

    expect(harness.depth()).toBe(0);
    expect(result.current.destination).toEqual({ type: 'tab', tab: 'home' });
    expect(result.current.canGoBack).toBe(false);
  });

  it('keeps depth honest when a new destination replaces forward entries', () => {
    const { result } = renderHook(() => useDestination());

    act(() => result.current.navigateToTab('browse'));
    act(() => result.current.navigateToModDetail('sodium'));
    act(() => result.current.goBack());
    act(() => result.current.navigateToInstanceDetail('inst-1'));

    expect(result.current.destination).toEqual({ type: 'instance-detail', instanceId: 'inst-1' });
    expect(harness.depth()).toBe(2);

    act(() => result.current.goBack());
    expect(result.current.destination).toEqual({ type: 'tab', tab: 'browse' });
  });

  it('falls back to home when history holds something that is not ours', () => {
    const { result } = renderHook(() => useDestination());

    act(() => result.current.navigateToTab('browse'));
    act(() => {
      window.dispatchEvent(new PopStateEvent('popstate', { state: { somethingElse: true } }));
    });

    expect(result.current.destination).toEqual({ type: 'tab', tab: 'home' });
    expect(result.current.canGoBack).toBe(false);
  });
});
