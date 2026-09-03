import { useEffect, useState } from 'react';
import {
  formatError,
  getMigrationReport,
  listManifestMcVersions,
  planVersionMigration,
  runVersionMigration,
  type MigrationPlan,
  type MigrationReport,
  type MigrationStatus,
  type ModMigrationEntry,
} from '@/lib/tauri';

const STATUS_LABEL: Record<MigrationStatus, string> = {
  ready: 'Ready',
  not_yet: 'No build yet',
  abandoned: 'Looks abandoned',
  superseded: 'Has a replacement',
  unknown: 'Could not check',
  unclassifiable: 'Needs a look',
};

const STATUS_ORDER: MigrationStatus[] = [
  'abandoned',
  'superseded',
  'not_yet',
  'unknown',
  'unclassifiable',
  'ready',
];

const VERDICT_TEXT: Record<MigrationReport['verdict'], string> = {
  ready: 'Everything has a build for the target version.',
  not_yet: 'Some mods have no build for the target version yet. Waiting may be enough.',
  blocked: 'Some mods look abandoned or have been replaced. Moving would leave gaps.',
  unknown: 'Some mods could not be checked, so this report is incomplete.',
  needs_review: 'Some items have no online identity to check. Have a look at those yourself.',
};

/**
 * "Can this pack move to the next Minecraft version?"
 *
 * The honest-reporting rule matters more than the happy path here: a mod that
 * could not be checked is reported as unchecked, never as dead, and one unknown
 * is enough to stop the whole report claiming to be definitive.
 */
export function MigrationReportPanel({
  instanceId,
  currentVersion,
  loader,
}: {
  instanceId: string;
  currentVersion: string;
  /** The instance's loader, so the target list only offers versions it has a
   *  build for. Omitted falls back to every known version. */
  loader?: string;
}) {
  const [target, setTarget] = useState('');
  const [report, setReport] = useState<MigrationReport | null>(null);
  const [plan, setPlan] = useState<MigrationPlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [versions, setVersions] = useState<string[]>([]);

  // The signed loader manifests already know every version, and which of them
  // the instance's loader supports — so this is a list to pick from, not a
  // string to spell correctly. Scoped by loader because offering a version
  // Fabric has no build for would only produce a report saying so.
  useEffect(() => {
    let cancelled = false;
    void listManifestMcVersions(loader)
      .then((list) => {
        if (cancelled) return;
        // The version you are already on is not somewhere to move to.
        setVersions(list.filter((version) => version !== currentVersion));
      })
      .catch(() => { if (!cancelled) setVersions([]); });
    return () => { cancelled = true; };
  }, [loader, currentVersion]);

  const run = async () => {
    const version = target.trim();
    if (!version) return;
    setBusy(true);
    setError(null);
    setStatus(null);
    setPlan(null);
    try {
      setReport(await getMigrationReport(instanceId, version));
      // A plan is the only thing that knows what will actually be left behind;
      // the report alone cannot say. A rejection here is not an error worth
      // shouting about — it usually just means the instance is still locked.
      try {
        setPlan(await planVersionMigration(instanceId, version));
      } catch {
        setPlan(null);
      }
    } catch (e) {
      setError(formatError(e));
      setReport(null);
    } finally {
      setBusy(false);
    }
  };

  const migrate = async () => {
    if (!plan) return;
    const leaving = plan.blockers;
    const prompt = leaving.length === 0
      ? `Move this instance to ${plan.targetVersion}? A snapshot is taken first, and a failed migration rolls back.`
      : `Move to ${plan.targetVersion} and leave ${leaving.length} item${leaving.length === 1 ? '' : 's'} at the current version?\n\n`
        + leaving.map((reason) => `• ${reason.message}`).join('\n');
    if (!confirm(prompt)) return;

    setBusy(true);
    setError(null);
    try {
      const outcome = await runVersionMigration(instanceId, plan.targetVersion, leaving.length > 0);
      switch (outcome.type) {
        case 'migrated':
          setStatus(`Now on ${outcome.toVersion}. ${outcome.replaced.length} item(s) replaced. Recovery snapshot: ${outcome.snapshotId}`);
          setReport(null);
          setPlan(null);
          break;
        case 'blocked':
          setError(outcome.reasons.map((reason) => reason.message).join('; '));
          break;
        case 'rolled-back':
          setError(`Migration failed during ${outcome.phase} and was rolled back — the instance is as it was. ${outcome.error}`);
          break;
        case 'failed':
          setError(outcome.rolledBack
            ? `Migration failed during ${outcome.phase} and was undone. ${outcome.error}`
            : `Migration failed during ${outcome.phase} and could NOT be undone automatically. ${outcome.error}`
              + (outcome.snapshotId ? ` Restore snapshot ${outcome.snapshotId} from the Snapshots tab.` : ''));
          break;
      }
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const byStatus = (status: MigrationStatus): ModMigrationEntry[] =>
    (report?.mods ?? []).filter((entry) => entry.status === status);

  return (
    <section className="rounded-xl border border-border bg-card p-4 space-y-4">
      <div>
        <h3 className="font-semibold text-sm">Move to a newer Minecraft version</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          Currently on {currentVersion}. Checking changes nothing — it reports whether every mod
          has a build for the version you name. Moving is a separate, confirmed step.
        </p>
      </div>

      <div className="flex gap-2">
        <select
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') void run(); }}
          aria-label="Target Minecraft version"
          disabled={versions.length === 0}
          className="rounded-lg border border-input bg-background px-3 py-2 text-sm w-56 disabled:opacity-50"
        >
          <option value="">
            {versions.length === 0 ? 'No other versions available' : 'Choose a version…'}
          </option>
          {versions.map((version) => (
            <option key={version} value={version}>{version}</option>
          ))}
        </select>
        <button
          type="button"
          onClick={() => void run()}
          disabled={busy || target.trim().length === 0}
          className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
        >
          {busy ? 'Checking…' : 'Check'}
        </button>
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}
      {status && <p className="text-sm text-primary">{status}</p>}

      {report && (
        <div className="space-y-3">
          <p className="text-sm">{VERDICT_TEXT[report.verdict]}</p>
          {/* Only worth printing when it is a genuine breakdown. When every mod
              is ready the verdict above already says so, and the disclosure
              below already carries the count — saying "3 ready of 3" between
              them was the same number a third time. */}
          {report.summary.ready !== report.summary.total && (
            <p className="text-xs text-muted-foreground">
              {report.summary.ready} ready
              {report.summary.not_yet > 0 && `, ${report.summary.not_yet} with no build yet`}
              {report.summary.abandoned > 0 && `, ${report.summary.abandoned} abandoned`}
              {report.summary.superseded > 0 && `, ${report.summary.superseded} replaced`}
              {report.summary.unknown > 0 && `, ${report.summary.unknown} unchecked`}
              {report.summary.unclassifiable > 0 && `, ${report.summary.unclassifiable} needing review`}
              {' '}of {report.summary.total}.
            </p>
          )}

          {report.warnings.map((warning) => (
            <p key={warning} className="text-xs text-muted-foreground">{warning}</p>
          ))}

          {plan && plan.warnings.map((warning) => (
            <p key={warning} className="text-xs text-muted-foreground">{warning}</p>
          ))}

          {plan && (
            <div className="flex flex-wrap items-center gap-2 border-t border-border pt-3">
              <button
                type="button"
                onClick={() => void migrate()}
                disabled={busy}
                className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              >
                {busy ? 'Migrating…' : `Move to ${plan.targetVersion}`}
              </button>
              <span className="text-xs text-muted-foreground">
                {plan.swaps.length} item{plan.swaps.length === 1 ? '' : 's'} will be replaced
                {plan.blockers.length > 0 && `, ${plan.blockers.length} left as ${plan.blockers.length === 1 ? 'it is' : 'they are'}`}.
                A snapshot is taken first.
              </span>
            </div>
          )}

          {!plan && report.verdict !== 'ready' && (
            <p className="text-xs text-muted-foreground">
              Unlock the instance to enable migrating it.
            </p>
          )}

          {STATUS_ORDER.map((status) => {
            const entries = byStatus(status);
            if (entries.length === 0) return null;
            return (
              <details key={status} open={status !== 'ready'}>
                <summary className="cursor-pointer text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                  {STATUS_LABEL[status]} ({entries.length})
                </summary>
                <ul className="mt-1 space-y-1">
                  {entries.map((entry) => (
                    <li key={entry.filename} className="text-sm">
                      <span>{entry.display_name}</span>
                      {entry.successor?.replacement_name && (
                        <span className="ml-2 text-xs text-muted-foreground">
                          → {entry.successor.replacement_name}
                        </span>
                      )}
                      {entry.error_message && (
                        <span className="ml-2 text-xs text-muted-foreground">
                          {entry.error_message}
                        </span>
                      )}
                    </li>
                  ))}
                </ul>
              </details>
            );
          })}
        </div>
      )}
    </section>
  );
}
