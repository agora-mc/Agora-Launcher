/**
 * AmbienceCoordinator — app-boundary bridge between the presentation
 * preference (interactive/live) and the ambience profile.
 *
 * The interactive layer must never import ambience (V5-PORT-PLAN §3: the
 * permitted outside-in dependency is `AmbienceProvider` reading settings).
 * This component lives at the app boundary, so it may touch both. It forces
 * the ambience profile for the current presentation:
 *
 *  - High Interaction forces at least `calm` and defaults to `full`, since
 *    the living world *is* that mode.
 *  - Simple mode leaves the ambience at the global setting (default off;
 *    `calm` remains available in Settings).
 */

import { useEffect, useRef } from 'react';
import { useAmbience } from '../features/ambience/AmbienceProvider';
import type { AmbienceProfile } from '../features/ambience/engine/engine';
import {
  loadPreference,
  PREFERENCE_CHANGED_EVENT,
  type InteractionPreference,
} from '../features/interactive/live/presentationPreference';
import { presentationCapabilities } from './presentation-capabilities';

export interface AmbienceCoordinatorProps {
  /** The currently active top-level tab. */
  activeTab: string;
}

function overrideFor(activeTab: string): AmbienceProfile | null {
  let pref: InteractionPreference;
  try {
    pref = loadPreference();
  } catch {
    return null;
  }
  const caps = presentationCapabilities(pref);
  if (pref === 'high-interaction' && activeTab === 'instances') {
    return caps.ambience;
  }
  // Standard / Simple: let the global setting stand.
  return null;
}

export function AmbienceCoordinator({ activeTab }: AmbienceCoordinatorProps) {
  const { overrideProfile } = useAmbience();
  const tabRef = useRef(activeTab);
  tabRef.current = activeTab;

  useEffect(() => {
    const apply = () => {
      overrideProfile(overrideFor(tabRef.current));
    };
    apply();
    window.addEventListener(PREFERENCE_CHANGED_EVENT, apply);
    return () => {
      window.removeEventListener(PREFERENCE_CHANGED_EVENT, apply);
      overrideProfile(null);
    };
  }, [overrideProfile]);

  // Re-apply whenever the active tab changes.
  useEffect(() => {
    overrideProfile(overrideFor(activeTab));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTab]);

  return null;
}
