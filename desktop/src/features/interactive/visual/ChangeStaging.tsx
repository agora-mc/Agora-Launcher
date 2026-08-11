import type { CapabilityFlags, VisualProposal, VisualScene } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { PhaseMark } from './primitives/stateMarks';
import { Announcement } from './primitives/announce';

/**
 * Change Staging — "What is true now, and what am I proposing?"
 *
 * A bordered "Proposed changes" dock with per-proposal phase marks and
 * destructive markers. This is a review visual: it never applies anything.
 * The host supplies `reviewLabel`/`onReview` (e.g. "Review in Standard flow"
 * in live mode, or "Apply simulated plan" in Lab) and owns what happens.
 *
 * A controlled component: emits `VisualIntent`s only, never calls Tauri.
 */

export interface ChangeStagingProps {
  scene: VisualScene;
  onIntent: (intent: VisualIntent) => void;
  capabilities: CapabilityFlags;
  /** Presentational label for the review control (defaults to "Review changes"). */
  reviewLabel?: string;
  /** Presentational availability gate; the control is hidden when false. */
  reviewAvailable?: boolean;
  /** When set, renders this applied outcome instead of the empty staging message. */
  outcome?: { title: string; summary: string; recoveryPoint?: string } | null;
  reducedMotion?: boolean;
}

export function ChangeStaging({
  scene,
  onIntent,
  reviewLabel = 'Review changes',
  reviewAvailable = true,
  outcome,
}: ChangeStagingProps) {
  const staged = scene.proposals.filter((proposal) => proposal.phase === 'proposed');
  const inFlight = scene.proposals.filter(
    (proposal) => proposal.phase === 'in-review' || proposal.phase === 'applying',
  );
  const destructiveCount = scene.proposals.filter((proposal) => proposal.destructive).length;
  const announcement = staged.length > 0
    ? `${staged.length} proposed change${staged.length === 1 ? '' : 's'} in staging`
    : null;

  return (
    <section
      aria-label="Change staging"
      className="rounded-xl border-2 border-dashed border-indigo-500/50 bg-indigo-500/5 p-4"
      data-source={scene.source.kind}
    >
      <Announcement message={announcement} />
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="flex items-center gap-2 text-base font-bold text-foreground">
          <span aria-hidden="true">▣</span> Proposed changes
          <span className="rounded-full bg-card px-2 py-0.5 text-xs font-semibold text-foreground">
            {staged.length + inFlight.length}
          </span>
        </h3>
        {destructiveCount > 0 ? (
          <span className="text-xs font-semibold text-destructive">
            includes {destructiveCount} destructive change{destructiveCount === 1 ? '' : 's'}
          </span>
        ) : null}
      </div>

      {scene.proposals.length === 0 ? (
        outcome ? (
          <div
            className="mt-3 rounded-lg border border-emerald-600/40 bg-emerald-600/5 p-3"
            role="region"
            aria-label="Applied outcome"
            data-outcome="applied"
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="text-sm font-bold text-foreground">{outcome.title}</span>
              <PhaseMark phase="committed" />
            </div>
            <p className="mt-1 text-sm text-muted-foreground">{outcome.summary}</p>
            {outcome.recoveryPoint ? (
              <p className="mt-1 text-xs font-medium text-foreground">Return point: {outcome.recoveryPoint}</p>
            ) : null}
          </div>
        ) : (
          <p className="mt-3 text-sm text-muted-foreground">No changes staged yet. Current state is unchanged.</p>
        )
      ) : (
        <ul className="mt-3 space-y-2">
          {scene.proposals.map((proposal: VisualProposal) => (
            <li
              key={proposal.id}
              className="rounded-lg border border-border bg-card p-3"
              data-proposal-phase={proposal.phase}
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <span className="text-sm font-bold text-foreground">{proposal.title}</span>
                <PhaseMark phase={proposal.phase} />
              </div>
              <p className="mt-1 text-sm text-muted-foreground">{proposal.summary}</p>
              {proposal.destructive ? (
                <p className="mt-1 text-xs font-semibold text-destructive">Destructive — affects existing content.</p>
              ) : null}
            </li>
          ))}
        </ul>
      )}

      {staged.length > 0 && reviewAvailable ? (
        <button
          type="button"
          onClick={() => onIntent({ kind: 'review-staged-changes' })}
          className="mt-4 rounded-md bg-primary px-3 py-1.5 text-sm font-semibold text-primary-foreground hover:opacity-90"
        >
          {reviewLabel}
        </button>
      ) : null}
    </section>
  );
}
