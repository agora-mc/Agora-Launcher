import type { CapabilityFlags, ExperienceSource, Severity, VisualHealthFinding, VisualId } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { isIntentEnabled } from '../domain/guards';
import { CompatibilityChip, SeverityBadge, StatusChip } from './primitives/statusChips';

/**
 * Health Lens — "What blocks launch, what is a warning, and what is merely advice?"
 *
 * Presents `VisualHealthFinding`s in the authoritative severity hierarchy
 * (blockers, warnings, recommendations). A recommendation never acquires
 * blocker styling. The `validated` flag distinguishes "no scan yet" from a
 * completed sweep. Findings may carry a `reviewIntent`, which this controlled
 * component emits — it never executes an operation.
 */

export interface HealthLensProps {
  findings: VisualHealthFinding[];
  source: ExperienceSource;
  validated: boolean;
  /** When true, the health read failed: nothing here may be treated as ready
   * (SOL-2 BLOCKER 1). */
  unavailable?: boolean;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  capabilities: CapabilityFlags;
  reducedMotion?: boolean;
}

const SEVERITY_ORDER: Severity[] = ['blocker', 'warning', 'recommendation'];

const KIND_LABEL: Record<string, string> = {
  'loader-compatibility': 'Loader',
  content: 'Content',
  runtime: 'Runtime',
  recovery: 'Recovery',
  other: 'Other',
};

function FindingRow({
  finding,
  source,
  selection,
  onSelect,
  onIntent,
  reviewEnabled,
}: {
  finding: VisualHealthFinding;
  source: ExperienceSource;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  reviewEnabled: boolean;
}) {
  const selected = selection === finding.id;
  return (
    <li
      className={`rounded-lg border bg-card p-3 ${selected ? 'border-primary ring-1 ring-primary/40' : 'border-border'}`}
      data-source={source.kind}
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <button
          type="button"
          onClick={() => onSelect(selected ? null : finding.id)}
          aria-pressed={selected}
          className="text-left"
        >
          <span className="block text-sm font-bold text-foreground">{finding.title}</span>
          <span className="text-xs text-muted-foreground">
            {/* "affects 0" reads as missing data; a finding that names no
                specific content is about the instance as a whole. */}
            {finding.structuredKind ? KIND_LABEL[finding.structuredKind] ?? finding.structuredKind : 'Health'}
            {finding.affectedIds.length > 0
              ? ` · affects ${finding.affectedIds.length}`
              : ' · this instance'}
          </span>
        </button>
        <SeverityBadge severity={finding.severity} />
      </div>
      <p className="mt-1 text-sm text-muted-foreground">{finding.summary}</p>
      {finding.suggestedAction ? (
        <p className="mt-1 text-xs font-medium text-foreground">Suggested: {finding.suggestedAction}</p>
      ) : null}
      {finding.compatibility ? (
        <div className="mt-1 flex items-center gap-2">
          <CompatibilityChip compatibility={finding.compatibility} />
        </div>
      ) : null}
      {finding.reviewIntent && reviewEnabled ? (
        <button
          type="button"
          onClick={() => onIntent(finding.reviewIntent!)}
          className="mt-2 rounded-md border border-border bg-muted/40 px-2.5 py-1 text-xs font-semibold text-foreground hover:bg-accent"
        >
          Review
        </button>
      ) : null}
    </li>
  );
}

export function HealthLens({
  findings,
  source,
  validated,
  unavailable = false,
  selection,
  onSelect,
  onIntent,
  capabilities,
}: HealthLensProps) {
  const blockers = findings.filter((finding) => finding.severity === 'blocker');
  const warnings = findings.filter((finding) => finding.severity === 'warning');
  const recommendations = findings.filter((finding) => finding.severity === 'recommendation');
  const total = findings.length;

  return (
    <section aria-label="Health lens" className="space-y-3" data-source={source.kind}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-base font-bold text-foreground">Health check</h3>
        <div className="flex items-center gap-2">
          {source.kind === 'simulation' ? <StatusChip label="Simulation" /> : null}
          {validated && total > 0 && isIntentEnabled({ kind: 'review-health' }, capabilities) ? (
            <button
              type="button"
              onClick={() => onIntent({ kind: 'review-health' })}
              className="rounded-md border border-border bg-card px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
            >
              Review health
            </button>
          ) : null}
        </div>
      </div>

      {unavailable ? (
        <p className="text-sm font-medium text-foreground" role="status">
          Health could not be verified right now — nothing here is treated as ready.
        </p>
      ) : validated ? (
        <div className="flex flex-wrap items-center gap-2 text-sm" role="status">
          <StatusChip label={`${blockers.length} blocker${blockers.length === 1 ? '' : 's'}`} />
          <StatusChip label={`${warnings.length} warning${warnings.length === 1 ? '' : 's'}`} />
          <StatusChip label={`${recommendations.length} recommendation${recommendations.length === 1 ? '' : 's'}`} />
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">
          Run the validation check to see what blocks launch, what needs review, and what is only advice.
        </p>
      )}

      {total === 0 && validated && !unavailable ? (
        <p className="text-sm font-medium text-foreground" role="status">
          No findings — this instance is ready to launch.
        </p>
      ) : null}

      {/* Findings are the RESULT of a scan: rendering them before one has run
          made the check button reveal nothing and put the outcome ahead of its
          cause (T6-8). The header above already explains what a scan will show. */}
      {(validated ? SEVERITY_ORDER : []).map((severity) => {
        const group = findings.filter((finding) => finding.severity === severity);
        if (group.length === 0) return null;
        const title =
          severity === 'blocker' ? 'Blockers' : severity === 'warning' ? 'Warnings' : 'Recommendations';
        return (
          <div key={severity}>
            <h4 className="mb-2 text-xs font-bold uppercase tracking-wide text-muted-foreground">{title}</h4>
            <ul className="space-y-2">
              {group.map((finding) => (
                <FindingRow
                  key={finding.id}
                  finding={finding}
                  source={source}
                  selection={selection}
                  onSelect={onSelect}
                  onIntent={onIntent}
                  reviewEnabled={finding.reviewIntent ? isIntentEnabled(finding.reviewIntent, capabilities) : false}
                />
              ))}
            </ul>
          </div>
        );
      })}

      {validated && total > 0 ? (
        <p className="text-xs text-muted-foreground">
          Blockers stop launch. Warnings need your review. Recommendations are advice — they never block.
        </p>
      ) : null}
    </section>
  );
}
