/**
 * Agora Lab shell — renders the Workshop.
 *
 * The Workshop is the whole Lab experience: a station map, six benches with
 * the four-step pedagogy, and per-step progress. This shell only wires the
 * app-level concerns:
 *  - Field Guide handoffs (the "Why" steps' real vocabulary)
 *  - Standard navigation for real destinations
 *  - the way out: the Lab is entered from the guide's "New to modding" tier
 *    rather than a sidebar tab, so the shell owns the return link
 *  - ambience: a bench drops the living background to `calm` while open, and
 *    restores it on close — in the Lab, motion is a teaching signal, so
 *    nothing should drift for decoration.
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
   * Return to the guide the Lab was opened from. The Lab has no sidebar tab,
   * so without this the only way back is another top-level destination.
   */
  onExit?: () => void;
  /**
   * A bench drops ambience to `calm` while open and restores the previous
   * profile (null) on close. The Lab itself never imports ambience; this
   * callback is the app-boundary wire.
   */
  onAmbienceChange?: (profile: 'calm' | null) => void;
}

export function LabShell({ onOpenGuide, onNavigateStandard, onExit, onAmbienceChange }: LabShellProps) {
  const handleOpenGuide = useCallback((topicId: string) => {
    onOpenGuide(topicId);
  }, [onOpenGuide]);

  return (
    <div className="w-full min-w-0 overflow-x-hidden" data-testid="lab-shell">
      {onExit && (
        <button
          type="button"
          onClick={onExit}
          className="mb-4 inline-flex items-center gap-2 rounded-lg border border-border bg-card px-3 py-1.5 text-sm font-semibold text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          {/* Inline arrow: `lab/` may not import icon packages (boundary
              allowlist in scripts/check-interactive-boundaries.mjs). */}
          <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            <path d="m12 19-7-7 7-7" />
            <path d="M19 12H5" />
          </svg>
          Back to Help &amp; Guide
        </button>
      )}
      <Workshop
        onOpenGuide={handleOpenGuide}
        onNavigateStandard={onNavigateStandard}
        onAmbienceChange={onAmbienceChange}
      />
    </div>
  );
}
