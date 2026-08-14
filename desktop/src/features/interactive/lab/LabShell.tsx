/**
 * Agora Lab shell — renders the Workshop (the v5-lab port, V5-PORT-PLAN §12).
 *
 * The Workshop is the whole Lab experience: a station map, six benches with
 * the four-step pedagogy, and per-step progress. This shell only wires the
 * app-level concerns:
 *  - Field Guide handoffs (the "Why" steps' real vocabulary)
 *  - Standard navigation for real destinations
 *  - ambience: a bench drops the living background to `calm` while open, and
 *    restores it on close (V5-PORT-PLAN §12.1 — in the Lab, motion is a
 *    teaching signal, so nothing should drift for decoration).
 */

import { useCallback } from 'react';
import { Workshop } from './workshop/Workshop';
import type { StandardDestination } from '../domain/intents';

export interface LabShellProps {
  onOpenGuide: (topicId: string) => void;
  onNavigateStandard: (dest: StandardDestination) => void;
  /** Friendly labels for Field Guide topic ids (from GUIDE_TOPICS). */
  guideTopicLabels?: Record<string, string>;
  /**
   * V5-PORT-PLAN §12.1: a bench drops ambience to `calm` while open and
   * restores the previous profile (null) on close. The Lab itself never
   * imports ambience; this callback is the app-boundary wire.
   */
  onAmbienceChange?: (profile: 'calm' | null) => void;
}

export function LabShell({ onOpenGuide, onNavigateStandard, onAmbienceChange }: LabShellProps) {
  const handleOpenGuide = useCallback((topicId: string) => {
    onOpenGuide(topicId);
  }, [onOpenGuide]);

  return (
    <div className="w-full min-w-0 overflow-x-hidden" data-testid="lab-shell">
      <Workshop
        onOpenGuide={handleOpenGuide}
        onNavigateStandard={onNavigateStandard}
        onAmbienceChange={onAmbienceChange}
      />
    </div>
  );
}
