/**
 * PresentationMotionCoordinator — app-boundary bridge between the interaction
 * preference and the app-wide motion preference.
 *
 * Sibling of `AmbienceCoordinator`, and there for the same reason: the
 * interactive layer must never reach into theme state, and the theme provider
 * must never depend on an interactive feature. This component sits above both
 * and applies the one cross-cutting rule the caps table declares — Simple mode
 * pins motion to `reduced`.
 *
 * The rule is CONTINUOUS, not a one-shot write at selection time: a Simple
 * session saved before the rule existed (or a preferences reset) must still end
 * up reduced, and Settings disables the Motion control while Simple is active
 * so the pin reads as a pin rather than a silent override.
 *
 * The rest of the cascade is downstream and lives where it belongs: reduced
 * motion switches the living background off inside `AmbienceProvider`, and the
 * living background going off takes background removal with it.
 */

import { useEffect, useRef } from 'react';
import { useUiPreferences } from './theme/theme-provider';
import {
  loadPreference,
  PREFERENCE_CHANGED_EVENT,
} from '../features/interactive/live/presentationPreference';
import { pinnedMotion } from './presentation-capabilities';

export function PresentationMotionCoordinator() {
  const { preferences, setPreferences } = useUiPreferences();

  // `setPreferences` has a fresh identity every provider render, so it is held
  // in a ref: the listener is registered once instead of re-subscribing on
  // every unrelated appearance tweak.
  const applyRef = useRef<() => void>(() => {});
  applyRef.current = () => {
    let pinned: ReturnType<typeof pinnedMotion>;
    try {
      pinned = pinnedMotion(loadPreference());
    } catch {
      return; // storage unavailable — leave the user's motion setting alone
    }
    if (pinned && preferences.motion !== pinned) setPreferences({ motion: pinned });
  };

  useEffect(() => {
    const apply = () => applyRef.current();
    apply();
    window.addEventListener(PREFERENCE_CHANGED_EVENT, apply);
    return () => window.removeEventListener(PREFERENCE_CHANGED_EVENT, apply);
  }, []);

  // Re-assert the pin whenever the motion preference changes underneath it
  // (e.g. an appearance preset writing `motion: 'full'` while Simple is on).
  useEffect(() => { applyRef.current(); }, [preferences.motion]);

  return null;
}
