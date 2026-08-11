import type { CapabilityFlags, ExperienceSource, VisualId, VisualNetworkReadiness } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { StatusChip } from './primitives/statusChips';

/**
 * Network Readiness Map — "Which player-visible needs are verified for offline use?"
 *
 * Presents a `VisualNetworkReadiness` with player-visible categories (game
 * files, loader, content, Java, sign-in/launch) and explicit states: Ready,
 * Missing, Blocked by policy, Unknown. Unknown is never promoted to ready, and
 * a cached catalog never lights all nodes by itself. `review-offline-readiness`
 * is the only intent emitted (a host-owned re-check).
 */

export interface NetworkReadinessMapProps {
  readiness: VisualNetworkReadiness;
  source: ExperienceSource;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  capabilities: CapabilityFlags;
  reducedMotion?: boolean;
}

const CATEGORY_LABEL: Record<string, string> = {
  'game-files': 'Game files',
  loader: 'Loader',
  content: 'Content',
  java: 'Java',
  'sign-in-and-launch': 'Sign-in & launch',
};

const STATE_CHIP: Record<string, { label: string; className: string }> = {
  ready: { label: 'Ready', className: 'border-emerald-600/60 bg-emerald-600/10 text-emerald-700 dark:text-emerald-300' },
  missing: { label: 'Missing', className: 'border-destructive/70 bg-destructive/10 text-destructive' },
  'blocked-by-policy': { label: 'Blocked by policy', className: 'border-amber-500/70 bg-amber-500/10 text-amber-700 dark:text-amber-300' },
  unknown: { label: 'Unknown', className: 'border-border bg-muted/60 text-muted-foreground' },
};

const OVERALL_LABEL: Record<string, string> = {
  ready: 'Ready',
  'needs-attention': 'Needs attention',
  checking: 'Checking…',
  unknown: 'Unknown',
};

export function NetworkReadinessMap({
  readiness,
  source,
  onIntent,
  capabilities,
}: NetworkReadinessMapProps) {
  return (
    <section aria-label="Network readiness map" className="space-y-3" data-source={source.kind}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-base font-bold text-foreground">Offline readiness — {readiness.instanceName}</h3>
        <StatusChip label={`Launch: ${readiness.launchMode === 'delegated' ? 'Delegated' : readiness.launchMode === 'direct' ? 'Direct' : 'Unknown'}`} />
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <StatusChip label={`Overall: ${OVERALL_LABEL[readiness.overall] ?? readiness.overall}`} />
        <StatusChip label={`Policy: ${readiness.policy === 'lockdown' ? 'Lockdown' : readiness.policy === 'restricted' ? 'Restricted' : 'Normal'}`} />
        {readiness.checkedAt ? <StatusChip label={`Checked ${readiness.checkedAt}`} /> : null}
      </div>

      <ul className="space-y-2" aria-label="Readiness checks">
        {readiness.checks.map((check) => {
          const state = STATE_CHIP[check.state];
          return (
            <li key={check.id} className="rounded-lg border border-border bg-card px-3 py-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <span className="text-sm font-medium text-foreground">
                  {CATEGORY_LABEL[check.category] ?? check.category} — {check.label}
                </span>
                {state ? (
                  <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[0.7rem] font-semibold ${state.className}`}>
                    {state.label}
                  </span>
                ) : null}
              </div>
              <p className="mt-1 text-xs text-muted-foreground">{check.summary}</p>
            </li>
          );
        })}
      </ul>

      {capabilities.canReviewOfflineReadiness ? (
        <button
          type="button"
          onClick={() => onIntent({ kind: 'review-offline-readiness' })}
          className="rounded-md bg-primary px-3 py-1.5 text-sm font-semibold text-primary-foreground hover:opacity-90"
        >
          Re-check readiness
        </button>
      ) : null}

      <p className="text-xs text-muted-foreground">
        A cached catalog is not the same as ready. Unknown means Agora cannot verify it without making a change.
      </p>
    </section>
  );
}
