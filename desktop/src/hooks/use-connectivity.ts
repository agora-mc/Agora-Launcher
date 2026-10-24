import { useCallback, useEffect, useRef, useState } from 'react';
import { getConnectivityStatus, type ConnectivityState } from '@/lib/tauri';

/** How often the backend's request outcome is re-read while the app is open. */
const POLL_MS = 10_000;

export interface Connectivity {
  state: ConnectivityState;
  /** Unix seconds of the request that set `state`, or null if none has finished. */
  lastOutcomeAt: number | null;
}

/**
 * What the app's own recent requests reported, as the backend records them.
 *
 * This replaces `navigator.onLine`, which only says whether a network interface
 * is up. A proxy or firewall can leave that "online" while every request fails,
 * so the Privacy badge said "Online" during failures that said "offline". A
 * status that cannot be read is `unknown`, never `reachable`.
 */
export function useConnectivity(): Connectivity {
  const [connectivity, setConnectivity] = useState<Connectivity>({
    state: 'unknown',
    lastOutcomeAt: null,
  });

  // The timer and window events can overlap reads; only the newest may land,
  // so a slow older reply never overwrites a newer one.
  const latestRead = useRef(0);

  const read = useCallback(async () => {
    const id = ++latestRead.current;
    try {
      const snapshot = await getConnectivityStatus();
      if (id !== latestRead.current) return;
      setConnectivity({
        state: snapshot?.state ?? 'unknown',
        lastOutcomeAt: snapshot?.last_outcome_at ?? null,
      });
    } catch {
      if (id !== latestRead.current) return;
      setConnectivity({ state: 'unknown', lastOutcomeAt: null });
    }
  }, []);

  useEffect(() => {
    void read();
    const timer = window.setInterval(() => void read(), POLL_MS);
    window.addEventListener('focus', read);
    window.addEventListener('online', read);
    window.addEventListener('offline', read);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener('focus', read);
      window.removeEventListener('online', read);
      window.removeEventListener('offline', read);
    };
  }, [read]);

  return connectivity;
}
