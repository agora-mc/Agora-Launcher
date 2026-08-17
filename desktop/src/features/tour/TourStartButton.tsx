/**
 * The one control that turns the walkthrough on.
 *
 * Dropped into Settings, Help & Guide, and the home dashboard so the tour can
 * be started from wherever the user notices they want it. It renders nothing
 * outside a `TourProvider`, and turns into an "End the walkthrough" control
 * while a tour is running so no entry point can start a second one.
 */

import { useTour } from './TourProvider';

interface TourStartButtonProps {
  className?: string;
  /** Overrides the idle-state label. */
  label?: string;
}

const BASE_CLASS =
  'inline-flex items-center justify-center gap-2 rounded-lg px-4 py-2 text-sm font-semibold';

export function TourStartButton({ className, label }: TourStartButtonProps) {
  const tour = useTour();
  if (!tour) return null;

  if (tour.running) {
    return (
      <button
        type="button"
        onClick={tour.end}
        className={`${BASE_CLASS} border border-border bg-background text-foreground hover:bg-accent ${className ?? ''}`}
      >
        End the walkthrough
      </button>
    );
  }

  return (
    <button
      type="button"
      onClick={tour.start}
      className={`${BASE_CLASS} bg-primary text-primary-foreground hover:bg-primary/90 ${className ?? ''}`}
    >
      {label ?? (tour.completed ? 'Run the walkthrough again' : 'Start the walkthrough')}
    </button>
  );
}
