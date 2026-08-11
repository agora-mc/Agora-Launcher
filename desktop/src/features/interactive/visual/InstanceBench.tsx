import type { Compatibility, ExperienceSource, VisualId, VisualInstance } from '../domain/models';
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

/**
 * Compatibility wording for the CURRENT loader (SOL §22.3).
 *
 * Previously only `compatible` produced a hint, so `incompatible`, `unknown`,
 * and `indeterminate` all rendered as silence — visually identical to a
 * verified-good loader. On the live surface `unknown` is the only value the
 * read adapter ever produces, so "no marking" was the normal case for every
 * real instance. Uncertainty must read as uncertainty, and must stay distinct
 * from incompatibility. The bare "Unknown" chip is deliberately NOT reused
 * here: TERRA-5 found it ambiguous next to confident data.
 */
const LOADER_COMPATIBILITY_HINT: Record<Compatibility, { text: string; tone: 'muted' | 'bad' | 'caution' }> = {
  compatible: { text: 'Fits this setup', tone: 'muted' },
  incompatible: { text: 'Does not fit this setup', tone: 'bad' },
  indeterminate: { text: 'Needs review — not proven for this setup', tone: 'caution' },
  unknown: { text: 'Not verified', tone: 'muted' },
};

const HINT_TONE_CLASS = {
  muted: 'text-muted-foreground',
  bad: 'font-semibold text-destructive',
  caution: 'font-semibold text-amber-700 dark:text-amber-300',
} as const;

function FieldRow({
  label,
  current,
  proposed,
  hint,
  hintTone = 'muted',
}: {
  label: string;
  current: string;
  proposed?: string;
  hint?: string;
  hintTone?: 'muted' | 'bad' | 'caution';
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
      {hint ? <span className={`text-xs ${HINT_TONE_CLASS[hintTone]}`}>{hint}</span> : null}
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
      data-testid="instance-bench"
      data-launch-state={instance.launchState}
      data-lock-state={instance.lockState}
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
          hint={LOADER_COMPATIBILITY_HINT[loaderCurrent.compatibility].text}
          hintTone={LOADER_COMPATIBILITY_HINT[loaderCurrent.compatibility].tone}
        />
      </div>

      {loaderProposed ? (
        <div className="mt-2 flex items-center gap-2">
          <CompatibilityChip compatibility={loaderProposed.compatibility} />
          <span className="text-xs text-muted-foreground">proposed loader choice</span>
        </div>
      ) : null}

      <div className="mt-3 grid min-w-0 grid-cols-3 gap-2 text-center">
        <div className="min-w-0 rounded-lg bg-muted/40 p-2">
          <div className="text-lg font-bold text-foreground">{instance.contentSummary.enabled}</div>
          <div className="text-[0.7rem] font-medium text-muted-foreground">Enabled</div>
        </div>
        <div className="min-w-0 rounded-lg bg-muted/40 p-2">
          <div className="text-lg font-bold text-foreground">{instance.contentSummary.disabled}</div>
          <div className="text-[0.7rem] font-medium text-muted-foreground">Disabled</div>
        </div>
        <div className="min-w-0 rounded-lg bg-muted/40 p-2">
          <div className="text-lg font-bold text-foreground">{instance.contentSummary.needsAttention}</div>
          <div className="break-words text-[0.7rem] font-medium leading-tight text-muted-foreground">Need attention</div>
        </div>
      </div>
    </section>
  );
}
