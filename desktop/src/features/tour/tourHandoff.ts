/**
 * Onboarding → app-shell handoff.
 *
 * The onboarding wizard renders *instead of* the app shell, so it sits outside
 * the tour provider and cannot start a tour directly: the moment it finishes,
 * it unmounts. It leaves a note here instead, and the shell consumes the note
 * on its first render. The note is cleared as it is read, so a tour can never
 * start twice from one request.
 */

const PENDING_KEY = 'agora-tour-pending-start';

function getStorage(): Storage | null {
  try {
    if (typeof window !== 'undefined' && window.localStorage) return window.localStorage;
  } catch {
    // Storage access can throw outright under some privacy settings.
  }
  return null;
}

/** Ask the app shell to start the walkthrough as soon as it mounts. */
export function queueTourStart(): void {
  try {
    getStorage()?.setItem(PENDING_KEY, '1');
  } catch {
    // Without storage the user can still start the tour from Settings.
  }
}

/** Read and clear the request. Returns true when a start was queued. */
export function consumeQueuedTourStart(): boolean {
  const storage = getStorage();
  if (!storage) return false;
  try {
    const pending = storage.getItem(PENDING_KEY) === '1';
    if (pending) storage.removeItem(PENDING_KEY);
    return pending;
  } catch {
    return false;
  }
}
