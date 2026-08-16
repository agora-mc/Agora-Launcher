import type { ReactNode } from 'react';
import type { ProposalPhase } from '../../domain/models';
import type { ScenePhase } from '../../domain/state';

/**
 * Phase marks for the interactive state vocabulary.
 *
 * Meaning is carried by border treatment, icon, and text label — never color
 * alone.
 */

const PHASE_META: Record<
  ScenePhase | ProposalPhase,
  { label: string; className: string; mark: ReactNode }
> = {
  current: {
    label: 'Current',
    className: 'border border-border bg-card text-foreground',
    mark: <span aria-hidden="true">●</span>,
  },
  proposed: {
    label: 'Proposed',
    className: 'border border-dashed border-indigo-500/70 bg-indigo-500/5 text-foreground',
    mark: <span aria-hidden="true">◌</span>,
  },
  'in-review': {
    label: 'In review',
    className: 'border border-amber-500/70 bg-amber-500/10 text-foreground',
    mark: <span aria-hidden="true">◔</span>,
  },
  applying: {
    label: 'Applying',
    className: 'border border-amber-500/70 bg-amber-500/10 text-foreground',
    mark: <span aria-hidden="true">…</span>,
  },
  committed: {
    label: 'Committed',
    className: 'border border-emerald-500/70 bg-emerald-500/10 text-foreground',
    mark: <span aria-hidden="true">✓</span>,
  },
  rejected: {
    label: 'Rejected',
    className: 'border border-destructive/70 bg-destructive/10 text-foreground',
    mark: <span aria-hidden="true">✕</span>,
  },
};

export function PhaseMark({ phase }: { phase: ScenePhase | ProposalPhase }) {
  const meta = PHASE_META[phase];
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-md px-2 py-0.5 text-xs font-semibold ${meta.className}`}
      data-phase={phase}
    >
      <span className="text-[0.7rem] leading-none">{meta.mark}</span>
      <span>{meta.label}</span>
    </span>
  );
}

/** Marks a value as carrying a local proposed alternative to current. */
export function ProposedMark() {
  return (
    <span
      className="inline-flex items-center gap-1 rounded border border-dashed border-indigo-500/70 bg-indigo-500/5 px-1.5 py-0.5 text-[0.7rem] font-semibold text-foreground"
      data-phase="proposed"
    >
      <span aria-hidden="true">◌</span> Proposed
    </span>
  );
}
