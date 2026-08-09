import type { CapabilityFlags, ExperienceSource, VisualId, VisualSnapshot } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { StatusChip } from './primitives/statusChips';

/**
 * Recovery Timeline — "What safe return points exist, and what does each protect?"
 *
 * Return points appear in time order; current state is pinned separately.
 * Every point names its role, scope, and whether worlds/saves are included.
 * Restore is never a one-click playful gesture: selecting a point emits
 * `preview-snapshot`; a serious compare/confirm surface (host-owned) follows.
 *
 * A controlled component: emits `VisualIntent`s only, never calls Tauri.
 */

export interface RecoveryTimelineProps {
  snapshots: VisualSnapshot[];
  currentStateLabel: string;
  source: ExperienceSource;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  capabilities: CapabilityFlags;
  reducedMotion?: boolean;
}

const ROLE_LABEL: Record<VisualSnapshot['role'], string> = {
  manual: 'Manual snapshot',
  'known-good': 'Known good',
  'current-known-good': 'Current known good',
  'undo-restore': 'Undo restore point',
  automatic: 'Automatic return point',
};

function ScopeBadges({ snapshot }: { snapshot: VisualSnapshot }) {
  return (
    <span className="text-xs text-muted-foreground">
      {snapshot.protects.join(', ') || 'instance files'}
      {' — '}
      {snapshot.worldProtection === 'included'
        ? 'worlds included'
        : snapshot.worldProtection === 'not-included'
          ? 'worlds NOT included'
          : 'world inclusion unknown'}
    </span>
  );
}

export function RecoveryTimeline({
  snapshots,
  currentStateLabel,
  source,
  selection,
  onSelect,
  onIntent,
  capabilities,
}: RecoveryTimelineProps) {
  // Newest first, current state always pinned separately at the top.
  const ordered = [...snapshots].sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1));

  return (
    <section aria-label="Recovery timeline" className="space-y-3" data-source={source.kind}>
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="text-base font-bold text-foreground">Return points</h3>
        <StatusChip label="Current state" />
        <span className="text-sm font-medium text-foreground">{currentStateLabel}</span>
      </div>

      {ordered.length === 0 ? (
        <p className="text-sm text-muted-foreground">No return points yet.</p>
      ) : (
        <ol className="space-y-2">
          {ordered.map((snapshot, index) => {
            const selected = selection === snapshot.id;
            const isCurrentGood = snapshot.role === 'current-known-good';
            return (
              <li
                key={snapshot.id}
                className={`relative rounded-xl border bg-card p-3 ${selected ? 'border-primary ring-1 ring-primary/40' : 'border-border'} ${isCurrentGood ? 'border-emerald-600/50' : ''}`}
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <button
                    type="button"
                    onClick={() => onSelect(selected ? null : snapshot.id)}
                    aria-pressed={selected}
                    className="text-left"
                  >
                    <span className="block text-sm font-bold text-foreground">{snapshot.label}</span>
                    <span className="text-xs text-muted-foreground">
                      {ROLE_LABEL[snapshot.role]} · {snapshot.createdAt}
                    </span>
                  </button>
                  <span className="text-xs font-medium text-muted-foreground">{snapshot.sizeLabel}</span>
                </div>

                <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1">
                  <ScopeBadges snapshot={snapshot} />
                  {snapshot.changeSummary ? (
                    <span className="text-xs text-muted-foreground">
                      +{snapshot.changeSummary.added} ~{snapshot.changeSummary.changed} −{snapshot.changeSummary.removed}
                    </span>
                  ) : null}
                </div>

                {index < ordered.length - 1 ? (
                  <div aria-hidden="true" className="absolute -bottom-2 left-6 h-2 w-px bg-border" />
                ) : null}
              </li>
            );
          })}
        </ol>
      )}

      {selection ? (
        <div className="rounded-lg border border-dashed border-border bg-muted/30 p-3" role="region" aria-label="Return point comparison">
          <h4 className="text-sm font-bold text-foreground">Compare return point</h4>
          <p className="mt-1 text-xs text-muted-foreground">
            Open the compare view to see added, changed, and removed content plus the world/save boundary before anything is restored.
          </p>
          <div className="mt-2 flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => {
                const snapshot = ordered.find((candidate) => candidate.id === selection);
                if (snapshot) onIntent({ kind: 'preview-snapshot', snapshotId: snapshot.id });
              }}
              className="rounded-md border border-border bg-card px-2.5 py-1 text-xs font-semibold text-foreground hover:bg-accent"
            >
              Compare
            </button>
            {capabilities.canRequestSnapshotRestore ? (
              <button
                type="button"
                onClick={() => {
                  const snapshot = ordered.find((candidate) => candidate.id === selection);
                  if (snapshot) onIntent({ kind: 'request-snapshot-restore', snapshotId: snapshot.id });
                }}
                className="rounded-md border border-destructive/60 px-2.5 py-1 text-xs font-semibold text-destructive hover:bg-destructive/10"
              >
                Restore… (serious confirm follows)
              </button>
            ) : null}
          </div>
        </div>
      ) : null}
    </section>
  );
}
