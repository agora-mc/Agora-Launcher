/**
 * Remembers the motion preference a user had before Simple mode pinned it to
 * "reduced", so leaving Simple can put it back. Only the value Simple itself
 * changed is restored; anything the user set while another mode was active is
 * left alone.
 *
 * Per-device scratch in localStorage (it must survive a restart while Simple
 * stays on), never required: every call tolerates unavailable storage.
 */

import type { MotionPreference } from './theme/theme-provider';

export const MOTION_BEFORE_PIN_KEY = 'agora.motion-before-simple.v1';

const VALID: readonly MotionPreference[] = ['system', 'reduced', 'full'];

/** Save `motion` as the value to restore, unless one is already saved. */
export function rememberMotionBeforePin(motion: MotionPreference): void {
  try {
    if (window.localStorage.getItem(MOTION_BEFORE_PIN_KEY) !== null) return;
    window.localStorage.setItem(MOTION_BEFORE_PIN_KEY, motion);
  } catch {
    /* storage unavailable: nothing to restore later */
  }
}

/** Read and forget the saved value (null when none was saved). */
export function takeMotionBeforePin(): MotionPreference | null {
  try {
    const saved = window.localStorage.getItem(MOTION_BEFORE_PIN_KEY);
    if (saved === null) return null;
    window.localStorage.removeItem(MOTION_BEFORE_PIN_KEY);
    return VALID.includes(saved as MotionPreference) ? (saved as MotionPreference) : null;
  } catch {
    return null;
  }
}

/**
 * What motion should become when a pinning mode ends. `pinned` is the value the
 * mode imposed; if the current value differs, the user changed it since, so it
 * is kept.
 */
export function motionAfterPinEnds(
  current: MotionPreference,
  pinned: MotionPreference,
  saved: MotionPreference | null,
): MotionPreference {
  if (current !== pinned) return current;
  return saved ?? 'system';
}
