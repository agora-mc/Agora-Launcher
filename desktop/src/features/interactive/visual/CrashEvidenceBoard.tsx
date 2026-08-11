import type { CapabilityFlags, ExperienceSource, VisualCrashEvidence, VisualId } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { KnowledgeChip, StatusChip } from './primitives/statusChips';

/**
 * Crash Evidence Board — "What is the current hypothesis, and what evidence would test it?"
 *
 * An investigation metaphor: cards are clues, suspects are hypotheses with
 * strength (low/medium/high, never certainty), and an experiment is a bounded,
 * recoverable one-variable test. No outcome is presented as proof of a cause.
 * A controlled component: emits `VisualIntent`s only.
 */

export interface CrashEvidenceBoardProps {
  evidence: VisualCrashEvidence;
  source: ExperienceSource;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  capabilities: CapabilityFlags;
  reducedMotion?: boolean;
}

const EVIDENCE_KIND_LABEL: Record<string, string> = {
  'crash-report': 'Crash report',
  log: 'Log',
  'process-outcome': 'Process outcome',
  health: 'Health',
};

const STRENGTH_LABEL: Record<string, string> = {
  low: 'Low',
  medium: 'Medium',
  high: 'High',
};

const HYPOTHESIS_STATE_LABEL: Record<string, string> = {
  candidate: 'Candidate',
  testing: 'Testing',
  'less-likely': 'Less likely',
  inconclusive: 'Inconclusive',
};

const EXPERIMENT_PHASE_LABEL: Record<string, string> = {
  'read-only': 'Read-only',
  proposed: 'Proposed',
  running: 'Running',
  'awaiting-player-confirmation': 'Awaiting your confirmation',
  complete: 'Complete',
};

export function CrashEvidenceBoard({
  evidence,
  source,
  selection,
  onSelect,
  onIntent,
  capabilities,
}: CrashEvidenceBoardProps) {
  return (
    <section aria-label="Crash evidence board" className="space-y-3" data-source={source.kind}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-base font-bold text-foreground">{evidence.incidentLabel}</h3>
        <StatusChip label="Investigation" />
      </div>

      <div role="region" aria-label="Evidence sources">
        <h4 className="mb-2 text-xs font-bold uppercase tracking-wide text-muted-foreground">Clues</h4>
        <ul className="space-y-2">
          {evidence.evidenceSources.map((sourceItem, index) => (
            <li key={index} className="flex flex-wrap items-center gap-2 rounded-lg border border-border bg-card px-3 py-2">
              <span className="text-sm font-medium text-foreground">
                {EVIDENCE_KIND_LABEL[sourceItem.kind] ?? sourceItem.kind}
              </span>
              <KnowledgeChip knowledge={sourceItem.state} />
              <span className="text-xs text-muted-foreground">{sourceItem.summary}</span>
            </li>
          ))}
        </ul>
      </div>

      <div role="region" aria-label="Hypotheses">
        <h4 className="mb-2 text-xs font-bold uppercase tracking-wide text-muted-foreground">Hypotheses</h4>
        <ul className="space-y-2">
          {evidence.hypotheses.map((hypothesis) => {
            const selected = selection === hypothesis.id;
            return (
              <li
                key={hypothesis.id}
                className={`rounded-xl border bg-card p-3 ${selected ? 'border-primary ring-1 ring-primary/40' : 'border-border'}`}
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <button
                    type="button"
                    onClick={() => onSelect(selected ? null : hypothesis.id)}
                    aria-pressed={selected}
                    className="text-left"
                  >
                    <span className="block text-sm font-bold text-foreground">{hypothesis.title}</span>
                    <span className="text-xs text-muted-foreground">
                      Strength: {STRENGTH_LABEL[hypothesis.strength] ?? hypothesis.strength} · {HYPOTHESIS_STATE_LABEL[hypothesis.state] ?? hypothesis.state}
                    </span>
                  </button>
                </div>
                {hypothesis.supportingClues.length > 0 ? (
                  <p className="mt-1 text-xs text-muted-foreground">
                    Supports: {hypothesis.supportingClues.join('; ')}
                  </p>
                ) : null}
                {hypothesis.contradictoryClues.length > 0 ? (
                  <p className="mt-1 text-xs text-muted-foreground">
                    Contradicts: {hypothesis.contradictoryClues.join('; ')}
                  </p>
                ) : null}
              </li>
            );
          })}
        </ul>
      </div>

      <div className="rounded-lg border border-dashed border-border bg-muted/30 p-3" role="region" aria-label="Experiment">
        <div className="flex flex-wrap items-center gap-2">
          <h4 className="text-sm font-bold text-foreground">Experiment</h4>
          <StatusChip label={EXPERIMENT_PHASE_LABEL[evidence.experiment.phase] ?? evidence.experiment.phase} />
          {evidence.experiment.recoveryReady ? <StatusChip label="Recovery point ready" /> : null}
        </div>
        {evidence.experiment.summary ? (
          <p className="mt-1 text-xs text-muted-foreground">{evidence.experiment.summary}</p>
        ) : null}
        <p className="mt-1 text-xs text-muted-foreground">
          One change at a time. One launch does not prove a cause.
        </p>
        {capabilities.canOpenCrashDoctor ? (
          <button
            type="button"
            onClick={() => onIntent({ kind: 'open-crash-doctor' })}
            className="mt-2 rounded-md border border-border bg-card px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
          >
            Open Crash Doctor
          </button>
        ) : null}
      </div>

      <p className="text-xs text-muted-foreground">{evidence.privacyNote}</p>
    </section>
  );
}
