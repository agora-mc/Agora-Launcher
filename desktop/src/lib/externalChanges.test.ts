import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const listeners = vi.hoisted(() => new Map<string, (event: { payload: unknown }) => void>());
const unlisten = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, callback: (event: { payload: unknown }) => void) => {
    listeners.set(name, callback);
    return unlisten;
  }),
}));

import {
  EXTERNAL_CHANGE_EVENT,
  describeExternalChange,
  useExternalInstanceChanges,
} from './externalChanges';

describe('useExternalInstanceChanges', () => {
  beforeEach(() => {
    listeners.clear();
    unlisten.mockClear();
  });

  it('hands reported changes to the latest handler and stops listening on unmount', async () => {
    const first = vi.fn();
    const second = vi.fn();
    const { rerender, unmount } = renderHook(({ handler }) => useExternalInstanceChanges(handler), {
      initialProps: { handler: first },
    });
    await act(async () => {});
    rerender({ handler: second });

    const change = { instance_id: 'alpha', program: 'Agora CLI', operation: 'disable_mod' };
    act(() => listeners.get(EXTERNAL_CHANGE_EVENT)?.({ payload: [change] }));
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledWith([change]);

    act(() => listeners.get(EXTERNAL_CHANGE_EVENT)?.({ payload: [] }));
    expect(second).toHaveBeenCalledTimes(1);

    unmount();
    expect(unlisten).toHaveBeenCalled();
  });
});

describe('describeExternalChange', () => {
  it('names the program and the operation', () => {
    expect(describeExternalChange({ instance_id: 'a', program: 'Agora CLI', operation: 'disable_mod' }))
      .toBe('The Agora CLI changed this instance (disable mod). Showing the latest.');
    expect(describeExternalChange({ instance_id: 'a', program: '', operation: '' }))
      .toBe('Another Agora program changed this instance. Showing the latest.');
  });
});
