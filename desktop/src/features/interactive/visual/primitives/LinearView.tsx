import { useState } from 'react';

/**
 * Unified linear/spatial view switch for interactive visuals.
 *
 * Every canvas has a nearby "List view" or unified mode switch that does not
 * lose selection. The switch is a labeled button pair; switching never resets
 * selection.
 */

export function useLinearView(initial = false) {
  const [linear, setLinear] = useState(initial);
  return { linear, setLinear };
}

export function LinearViewToggle({
  linear,
  onChange,
  spatialLabel = 'Diagram view',
  linearLabel = 'List view',
}: {
  linear: boolean;
  onChange: (linear: boolean) => void;
  spatialLabel?: string;
  linearLabel?: string;
}) {
  return (
    <div className="inline-flex items-center gap-1 rounded-lg border border-border bg-muted/40 p-0.5" role="group" aria-label="View">
      <button
        type="button"
        onClick={() => onChange(false)}
        aria-pressed={!linear}
        className={`rounded-md px-2.5 py-1 text-xs font-semibold transition-colors ${!linear ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}
      >
        {spatialLabel}
      </button>
      <button
        type="button"
        onClick={() => onChange(true)}
        aria-pressed={linear}
        className={`rounded-md px-2.5 py-1 text-xs font-semibold transition-colors ${linear ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}
      >
        {linearLabel}
      </button>
    </div>
  );
}
