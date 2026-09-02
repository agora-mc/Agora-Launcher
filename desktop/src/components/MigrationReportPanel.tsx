import { useState } from 'react';
import {
  formatError,
  getMigrationReport,
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
}: {
  instanceId: string;
  currentVersion: string;
}) {
  const [target, setTarget] = useState('');
  const [report, setReport] = useState<MigrationReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async () => {
    const version = target.trim();
    if (!version) return;
    setBusy(true);
    setError(null);
    try {
      setReport(await getMigrationReport(instanceId, version));
    } catch (e) {
      setError(formatError(e));
      setReport(null);
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
          Currently on {currentVersion}. Nothing is changed — this only checks whether every mod
          has a build for the version you name.
        </p>
      </div>

      <div className="flex gap-2">
        <input
          type="text"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') void run(); }}
          placeholder="Target version, e.g. 1.21.4"
          aria-label="Target Minecraft version"
          className="rounded-lg border border-input bg-background px-3 py-2 text-sm w-56"
        />
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

      {report && (
        <div className="space-y-3">
          <p className="text-sm">{VERDICT_TEXT[report.verdict]}</p>
          <p className="text-xs text-muted-foreground">
            {report.summary.ready} ready
            {report.summary.not_yet > 0 && `, ${report.summary.not_yet} with no build yet`}
            {report.summary.abandoned > 0 && `, ${report.summary.abandoned} abandoned`}
            {report.summary.superseded > 0 && `, ${report.summary.superseded} replaced`}
            {report.summary.unknown > 0 && `, ${report.summary.unknown} unchecked`}
            {report.summary.unclassifiable > 0 && `, ${report.summary.unclassifiable} needing review`}
            {' '}of {report.summary.total}.
          </p>

          {report.warnings.map((warning) => (
            <p key={warning} className="text-xs text-muted-foreground">{warning}</p>
          ))}

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
