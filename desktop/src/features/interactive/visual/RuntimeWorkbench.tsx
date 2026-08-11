import type { CapabilityFlags, ExperienceSource, VisualId, VisualRuntimeState } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { CompatibilityChip, StatusChip } from './primitives/statusChips';
import { PhaseMark, ProposedMark } from './primitives/stateMarks';

/**
 * Runtime Workbench — "What is Agora choosing automatically, and what would my manual change do?"
 *
 * Shows the current runtime label and required Java generation, the current /
 * recommended / proposed memory choice with headroom, and the garbage-collector
 * mode. Automatic and manual are peer choices. Staging a manual memory value
 * emits `propose-memory`; the host owns review/save.
 */

export interface RuntimeWorkbenchProps {
  runtime: VisualRuntimeState;
  source: ExperienceSource;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  capabilities: CapabilityFlags;
  reducedMotion?: boolean;
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2">
      <span className="w-40 shrink-0 text-xs font-semibold text-muted-foreground">{label}</span>
      {children}
    </div>
  );
}

export function RuntimeWorkbench({
  runtime,
  source,
  onIntent,
  capabilities,
}: RuntimeWorkbenchProps) {
  const memoryMode = runtime.memory.mode;
  const memoryCurrent = memoryMode.current;
  const memoryProposed = memoryMode.proposed;
  const hasProposed = memoryProposed !== undefined;

  return (
    <section aria-label="Runtime workbench" className="space-y-3" data-source={source.kind}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-base font-bold text-foreground">Runtime</h3>
        {source.kind === 'simulation' ? <StatusChip label="Simulation" /> : null}
      </div>

      {runtime.availability === 'unavailable' ? (
        <p className="text-sm font-medium text-muted-foreground" role="status">
          Runtime data could not be verified — nothing here is treated as current.
        </p>
      ) : (
      <div className="divide-y divide-border rounded-xl border border-border bg-card px-3">
        <Row label="Runtime">
          <span className="rounded-md border border-border bg-card px-2 py-0.5 text-sm font-medium text-foreground">
            {runtime.runtime.currentLabel}
          </span>
          {runtime.runtime.requiredJavaMajor ? (
            <span className="text-xs text-muted-foreground">needs Java {runtime.runtime.requiredJavaMajor}+</span>
          ) : null}
          {runtime.runtime.compatibility === 'unknown' ? (
            // Indeterminate by design (no Java-compatibility read exists), NOT a
            // failed read — so it is an explicit note tied to the Runtime row, never
            // a global-looking chip that could undercut the memory recommendation.
            <span
              className="text-xs text-muted-foreground"
              title="Agora could not verify this Java runtime against the instance requirements."
            >
              Java runtime: not verified
            </span>
          ) : (
            <CompatibilityChip compatibility={runtime.runtime.compatibility} />
          )}
          {runtime.runtime.managedByAgora ? <StatusChip label="Managed by Agora" /> : null}
        </Row>

        <Row label="Memory mode">
          <span className="rounded-md border border-border bg-card px-2 py-0.5 text-sm font-medium text-foreground">
            {memoryCurrent === 'automatic' ? 'Automatic' : 'Manual'}
          </span>
          {hasProposed ? (
            <>
              <span aria-hidden="true" className="text-xs text-muted-foreground">→</span>
              <span className="rounded-md border border-dashed border-indigo-500/70 bg-indigo-500/5 px-2 py-0.5 text-sm font-medium text-foreground">
                {memoryProposed === 'automatic' ? 'Automatic' : 'Manual'}
              </span>
              <ProposedMark />
            </>
          ) : null}
        </Row>

        <Row label="Memory">
          <span className="text-sm font-medium text-foreground">
            {Math.round(runtime.memory.currentMiB / 1024)} GB configured
          </span>
          {runtime.memory.recommendedMiB ? (
            <span className="text-xs text-muted-foreground">
              recommended {Math.round(runtime.memory.recommendedMiB / 1024)} GB
            </span>
          ) : null}
          {runtime.memory.proposedMiB ? (
            <span className="text-xs text-muted-foreground">
              proposed {Math.round(runtime.memory.proposedMiB / 1024)} GB
            </span>
          ) : null}
          {runtime.memory.safeHeadroomLabel ? (
            <span className="text-xs text-muted-foreground">{runtime.memory.safeHeadroomLabel}</span>
          ) : null}
        </Row>

        <Row label="Garbage collector">
          <span className="text-sm font-medium text-foreground">
            {runtime.garbageCollector.current.mode === 'automatic'
              ? 'Automatic'
              : `Manual — ${runtime.garbageCollector.current.label}`}
          </span>
        </Row>
      </div>
      )}

      <p className="text-sm text-muted-foreground">{runtime.memory.explanation}</p>

      {(memoryCurrent === 'automatic' && memoryProposed === undefined && capabilities.canProposeMemory) || hasProposed || memoryCurrent === 'manual' ? (
        <div className="flex flex-wrap gap-2">
          {(memoryCurrent === 'manual' || memoryProposed === 'manual') && capabilities.canProposeMemory ? (
            <button
              type="button"
              onClick={() => onIntent({ kind: 'propose-memory', mode: 'automatic' })}
              className="rounded-md border border-border bg-card px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
            >
              Use recommended (automatic)
            </button>
          ) : null}
          {memoryCurrent === 'automatic' && memoryProposed === undefined && capabilities.canProposeMemory ? (
            <button
              type="button"
              onClick={() => onIntent({ kind: 'propose-memory', mode: 'manual', memoryMiB: runtime.memory.recommendedMiB ?? 4096 })}
              className="rounded-md bg-primary px-3 py-1.5 text-sm font-semibold text-primary-foreground hover:opacity-90"
            >
              Stage manual choice
            </button>
          ) : null}
          {hasProposed ? <PhaseMark phase="proposed" /> : null}
          {!capabilities.canProposeMemory && (hasProposed || memoryCurrent === 'manual') ? (
            <span className="text-xs font-medium text-muted-foreground" role="status">
              Memory changes are not available here — review in Standard view.
            </span>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
