import type { ExperienceSource, VisualId, VisualInstance } from '../domain/models';
import type { CapabilityFlags } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { hasProposal } from '../domain/state';
import { CompatibilityChip, StatusChip } from './primitives/statusChips';
import { PhaseMark, ProposedMark } from './primitives/stateMarks';

/**
 * Instance Bench — "What kind of instance am I building or inspecting?"
 *
 * A controlled view over a single `VisualInstance`. Shows current vs proposed
 * for the assembly fields (name, game version, loader) with explicit labels;
 * meaning is never carried by color alone.
 *
 * This component never calls Tauri or a live controller; it only emits
 * `VisualIntent`s through `onIntent`.
 */

export interface InstanceBenchProps {
  instance: VisualInstance;
  source: ExperienceSource;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  capabilities: CapabilityFlags;
  /** Role label distinguishing this bench from siblings, e.g. "Your new instance". */
  roleLabel?: string;
  /** When set, replaces the Current/Proposed phase mark (e.g. "Separate · unchanged"). */
  statusLabel?: string;
  /** Emphasize this bench as the active/working instance. */
  highlight?: boolean;
  reducedMotion?: boolean;
}

function FieldRow({
  label,
  current,
  proposed,
  hint,
}: {
  label: string;
  current: string;
  proposed?: string;
  hint?: string;
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2">
      <span className="w-36 shrink-0 text-xs font-semibold text-muted-foreground">{label}</span>
      <span className="rounded-md border border-border bg-card px-2 py-0.5 text-sm font-medium text-foreground">
        {current}
      </span>
      {proposed !== undefined && (
        <>
          <span aria-hidden="true" className="text-xs text-muted-foreground">→</span>
          <span className="flex items-center gap-1.5">
            <span className="rounded-md border border-dashed border-indigo-500/70 bg-indigo-500/5 px-2 py-0.5 text-sm font-medium text-foreground">
              {proposed}
            </span>
            <ProposedMark />
          </span>
        </>
      )}
      {hint ? <span className="text-xs text-muted-foreground">{hint}</span> : null}
    </div>
  );
}

export function InstanceBench({
  instance,
  source,
  selection,
  onSelect,
  roleLabel,
  statusLabel,
  highlight,
}: InstanceBenchProps) {
  const selected = selection === instance.id;
  const loaderCurrent = instance.loader.current;
  const loaderProposed = instance.loader.proposed;

  return (
    <section
      aria-label={roleLabel ?? 'Instance bench'}
      className={`rounded-xl border bg-card p-4 ${selected ? 'border-primary ring-1 ring-primary/40' : highlight ? 'border-primary/60 ring-1 ring-primary/20' : 'border-border'} ${highlight ? '' : 'opacity-90'}`}
      data-source={source.kind}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3 className="text-base font-bold text-foreground">{roleLabel ?? 'Instance bench'}</h3>
          {roleLabel ? <p className="text-xs text-muted-foreground">Instance bench</p> : null}
        </div>
        <div className="flex items-center gap-2">
          {source.kind === 'simulation' ? <StatusChip label="Simulation" /> : null}
          {statusLabel ? (
            <StatusChip label={statusLabel} />
          ) : hasProposal({ current: loaderCurrent, proposed: loaderProposed }) ? (
            <PhaseMark phase="proposed" />
          ) : (
            <PhaseMark phase="current" />
          )}
        </div>
      </div>

      <button
        type="button"
        onClick={() => onSelect(selected ? null : instance.id)}
        aria-pressed={selected}
        className="mt-1 block w-full rounded-lg border border-transparent p-1 text-left hover:border-border"
      >
        <span className="text-xs font-semibold text-muted-foreground">Purpose / name</span>
        <span className="block text-lg font-bold text-foreground">{instance.name}</span>
      </button>

      <div className="mt-2 divide-y divide-border">
        <FieldRow
          label="Game version"
          current={instance.gameVersion}
          hint="Each instance keeps its own version."
        />
        <FieldRow
          label="Loader"
          current={loaderCurrent.family}
          proposed={loaderProposed ? `${loaderProposed.family}${loaderProposed.version ? ` ${loaderProposed.version}` : ''}` : undefined}
          hint={loaderCurrent.compatibility === 'compatible' ? 'Fits this setup' : undefined}
        />
      </div>

      {loaderProposed ? (
        <div className="mt-2 flex items-center gap-2">
          <CompatibilityChip compatibility={loaderProposed.compatibility} />
          <span className="text-xs text-muted-foreground">proposed loader choice</span>
        </div>
      ) : null}

      <div className="mt-3 grid grid-cols-3 gap-2 text-center">
        <div className="rounded-lg bg-muted/40 p-2">
          <div className="text-lg font-bold text-foreground">{instance.contentSummary.enabled}</div>
          <div className="text-[0.7rem] font-medium text-muted-foreground">Enabled</div>
        </div>
        <div className="rounded-lg bg-muted/40 p-2">
          <div className="text-lg font-bold text-foreground">{instance.contentSummary.disabled}</div>
          <div className="text-[0.7rem] font-medium text-muted-foreground">Disabled</div>
        </div>
        <div className="rounded-lg bg-muted/40 p-2">
          <div className="text-lg font-bold text-foreground">{instance.contentSummary.needsAttention}</div>
          <div className="text-[0.7rem] font-medium text-muted-foreground">Need attention</div>
        </div>
      </div>
    </section>
  );
}
