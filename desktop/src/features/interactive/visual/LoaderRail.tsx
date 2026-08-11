import type { CapabilityFlags, ExperienceSource, VisualId, VisualLoaderCandidate } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { CompatibilityChip, StatusChip } from './primitives/statusChips';

/**
 * Loader Rail — "Which loader choice is current, compatible, uncertain, or recommended?"
 *
 * Presents `VisualLoaderCandidate`s with explicit role (Current / Recommended /
 * Alternative), proven-vs-indeterminate compatibility, and requirement counts.
 * An indeterminate candidate is never styled as compatible. Selecting a
 * candidate emits `review-loader` — the host owns the review/confirmation.
 */

export interface LoaderRailProps {
  candidates: VisualLoaderCandidate[];
  source: ExperienceSource;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  capabilities: CapabilityFlags;
  reducedMotion?: boolean;
}

const ROLE_LABEL: Record<VisualLoaderCandidate['role'], string> = {
  current: 'Current',
  recommended: 'Recommended',
  alternative: 'Alternative',
};

const CHANNEL_LABEL: Record<VisualLoaderCandidate['channel'], string> = {
  stable: 'Stable',
  prerelease: 'Prerelease',
  unknown: 'Channel unknown',
};

export function LoaderRail({
  candidates,
  source,
  selection,
  onSelect,
  onIntent,
  capabilities,
}: LoaderRailProps) {
  return (
    <section aria-label="Loader rail" className="space-y-3" data-source={source.kind}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-base font-bold text-foreground">Loader choices</h3>
        <StatusChip label="Compatibility" />
      </div>

      {candidates.length === 0 ? (
        <p className="text-sm text-muted-foreground">No loader candidates are available for this instance.</p>
      ) : (
        <ul className="space-y-2">
          {candidates.map((candidate) => {
            const selected = selection === candidate.id;
            return (
              <li
                key={candidate.id}
                className={`rounded-xl border bg-card p-3 ${selected ? 'border-primary ring-1 ring-primary/40' : 'border-border'}`}
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <button
                    type="button"
                    onClick={() => onSelect(selected ? null : candidate.id)}
                    aria-pressed={selected}
                    className="text-left"
                  >
                    <span className="block text-sm font-bold text-foreground">
                      {candidate.family} {candidate.version}
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {CHANNEL_LABEL[candidate.channel]} · {ROLE_LABEL[candidate.role]}
                    </span>
                  </button>
                  <CompatibilityChip compatibility={candidate.compatibility} />
                </div>

                <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
                  <span className="text-muted-foreground">
                    {candidate.requirementSummary.satisfied} satisfied · {candidate.requirementSummary.indeterminate} uncertain · {candidate.requirementSummary.failed} failed
                  </span>
                  {candidate.affectedContent.visibleNames.length > 0 ? (
                    <span className="text-muted-foreground">
                      affects {candidate.affectedContent.visibleNames.slice(0, 3).join(', ')}
                      {candidate.affectedContent.total > 3 ? ` +${candidate.affectedContent.total - 3} more` : ''}
                    </span>
                  ) : null}
                </div>

                <p className="mt-1 text-xs text-muted-foreground">{candidate.explanation}</p>

                {capabilities.canReviewLoader ? (
                  <button
                    type="button"
                    onClick={() => onIntent({ kind: 'review-loader', candidateId: candidate.id })}
                    className="mt-2 rounded-md border border-border bg-muted/40 px-2.5 py-1 text-xs font-semibold text-foreground hover:bg-accent"
                  >
                    Review this loader
                  </button>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}

      <p className="text-xs text-muted-foreground">
        A proven-compatible choice is different from one that needs review. Uncertain is not compatible.
      </p>
    </section>
  );
}
