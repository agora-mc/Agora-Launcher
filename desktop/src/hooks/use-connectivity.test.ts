import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

type Snapshot = { state: string; last_outcome_at: number | null };
const pending = vi.hoisted(() => [] as Array<(value: Snapshot) => void>);
vi.mock('@/lib/tauri', () => ({
  getConnectivityStatus: vi.fn(
    () => new Promise<Snapshot>((resolve) => pending.push(resolve)),
  ),
}));

import { useConnectivity } from './use-connectivity';

describe('useConnectivity', () => {
  it('keeps the newest reading when an older read answers last', async () => {
    const { result, unmount } = renderHook(() => useConnectivity());
    // Mount starts read 1; a focus event starts read 2.
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    expect(pending).toHaveLength(2);

    await act(async () => {
      pending[1]({ state: 'reachable', last_outcome_at: 200 });
    });
    await act(async () => {
      pending[0]({ state: 'unreachable', last_outcome_at: 100 });
    });

    expect(result.current).toEqual({ state: 'reachable', lastOutcomeAt: 200 });
    unmount();
  });
});
