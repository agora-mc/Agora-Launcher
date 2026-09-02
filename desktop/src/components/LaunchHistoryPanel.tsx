import { useEffect, useState } from 'react';
import { formatError, getLaunchHistory, type LaunchHistoryView } from '@/lib/tauri';

function seconds(ms: number | null): string {
  if (ms == null) return '—';
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

function duration(ms: number | null): string {
  if (ms == null) return 'still running';
  const mins = Math.round(ms / 60000);
  return mins < 1 ? '<1 min' : `${mins} min`;
}

/**
 * Local launch history.
 *
 * Nothing here leaves the machine and there is no endpoint to send it to — it
 * lives in the same local database as everything else the user owns and is
 * deleted with the instance.
 */
export function LaunchHistoryPanel({ instanceId }: { instanceId: string }) {
  const [view, setView] = useState<LaunchHistoryView | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getLaunchHistory(instanceId)
      .then((result) => { if (!cancelled) setView(result); })
      .catch((e) => { if (!cancelled) setError(formatError(e)); });
    return () => { cancelled = true; };
  }, [instanceId]);

  const stats = view?.stats;
  const recent = stats?.recent_median_prep_ms;
  const earlier = stats?.earlier_median_prep_ms;
  // Only claim a change when both halves exist and the gap is worth mentioning.
  const trend = recent != null && earlier != null && earlier > 0
    ? (recent - earlier) / earlier
    : null;

  return (
    <section className="rounded-xl border border-border bg-card p-4 space-y-3">
      <div>
        <h3 className="font-semibold text-sm">Launch history</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          Recorded on this computer only. Never sent anywhere, and removed when you delete the
          instance.
        </p>
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}

      {stats && stats.runs === 0 && (
        <p className="text-sm text-muted-foreground">No launches recorded yet.</p>
      )}

      {stats && stats.runs > 0 && (
        <>
          <p className="text-sm">
            {stats.runs} launch{stats.runs === 1 ? '' : 'es'}
            {stats.crashes > 0 && `, ${stats.crashes} ended in a crash`}.
            {stats.median_prep_ms != null && ` Agora typically takes ${seconds(stats.median_prep_ms)} to get ready.`}
          </p>

          {trend != null && Math.abs(trend) >= 0.25 && (
            <p className="text-xs text-muted-foreground">
              Preparation is {trend > 0 ? 'slower' : 'faster'} lately — {seconds(recent!)} recently
              versus {seconds(earlier!)} before
              {stats.latest_mod_count != null && stats.earliest_mod_count != null
                && stats.latest_mod_count !== stats.earliest_mod_count
                && `, with ${stats.latest_mod_count} enabled mods now versus ${stats.earliest_mod_count} then`}.
            </p>
          )}

          <ul className="max-h-64 space-y-1 overflow-y-auto">
            {view!.records.map((record) => (
              <li key={record.id} className="flex items-center gap-3 text-xs">
                <span className="text-muted-foreground">
                  {new Date(record.started_at).toLocaleString()}
                </span>
                <span>{duration(record.duration_ms)}</span>
                <span className="text-muted-foreground">{record.enabled_mod_count} mods</span>
                <span className={`ml-auto ${record.outcome === 'crashed' ? 'text-destructive' : 'text-muted-foreground'}`}>
                  {record.outcome === 'crashed' ? 'crashed'
                    : record.outcome === 'ok' ? 'ok'
                    : record.outcome === 'unknown' ? 'unknown'
                    : 'running'}
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
