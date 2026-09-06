import { useState } from 'react';
import {
  applyPackUpdate,
  formatError,
  pickOpenFile,
  previewPackUpdate,
  type ConflictResolution,
  type PackUpdatePreview,
} from '@/lib/tauri';
import { useConfirm } from '@/components/ui/confirm';

function formatBytes(bytes: number): string {
  if (bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** exponent;
  return `${value >= 10 || exponent === 0 ? Math.round(value) : value.toFixed(1)} ${units[exponent]}`;
}

/**
 * Update a modpack without losing what the user changed.
 *
 * Every other launcher wipes and replaces, so added mods vanish, config edits
 * vanish, and deliberately-disabled mods come back on. This runs a three-way
 * merge instead — and where it genuinely cannot tell whose change should win,
 * it asks rather than guessing.
 */
export function PackUpdatePanel({ instanceId, locked }: { instanceId: string; locked: boolean }) {
  const { confirm } = useConfirm();
  const [mrpackPath, setMrpackPath] = useState<string | null>(null);
  const [preview, setPreview] = useState<PackUpdatePreview | null>(null);
  const [resolutions, setResolutions] = useState<Record<string, ConflictResolution>>({});
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const unresolved = (preview?.plan.conflicts ?? []).filter((c) => !resolutions[c.key]);

  const choose = async () => {
    setError(null);
    setStatus(null);
    const picked = await pickOpenFile('Choose the new pack file', ['mrpack']);
    if (!picked) return;
    setMrpackPath(picked);
    setPreview(null);
    setResolutions({});
    setBusy(true);
    try {
      setPreview(await previewPackUpdate(instanceId, picked));
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const apply = async () => {
    if (!preview || !mrpackPath) return;
    if (!await confirm({
      title: `Update to ${preview.packName}?`,
      body: 'A snapshot is taken first, and a failed update rolls back.',
      confirmLabel: 'Update pack',
    })) return;
    setBusy(true);
    setError(null);
    try {
      const outcome = await applyPackUpdate(instanceId, mrpackPath, resolutions);
      switch (outcome.type) {
        case 'updated':
          setStatus(`Updated. ${outcome.changed} change${outcome.changed === 1 ? '' : 's'} applied, ${outcome.kept} left as you had them. Recovery snapshot: ${outcome.snapshotId}`);
          setPreview(null);
          setMrpackPath(null);
          break;
        case 'health-blocked':
          setStatus(`Updated, but the health check found problems — your files are kept so you can look. Recovery snapshot: ${outcome.snapshotId}`);
          setPreview(null);
          break;
        case 'failed':
          setError(outcome.rolledBack
            ? `Update failed during ${outcome.phase} and was rolled back — nothing changed. ${outcome.error}`
            : `Update failed during ${outcome.phase}. ${outcome.error}`);
          break;
      }
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="rounded-xl border border-border bg-card p-4 space-y-4">
      <div>
        <h3 className="font-semibold text-sm">Update the modpack</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          Keeps the mods you added, the settings you changed, and the mods you turned off. Where
          both you and the pack changed the same file, you decide.
        </p>
      </div>

      <button
        type="button"
        onClick={() => void choose()}
        disabled={busy || locked}
        className="rounded-lg border border-input bg-background px-4 py-2 text-sm font-medium hover:bg-accent disabled:opacity-50"
      >
        {busy && !preview ? 'Reading…' : 'Choose new pack file…'}
      </button>

      {error && <p className="text-sm text-destructive">{error}</p>}
      {status && <p className="text-sm text-primary">{status}</p>}

      {preview && (
        <div className="space-y-3">
          <p className="text-sm">
            {preview.packName}
            {preview.packVersionId && <span className="text-muted-foreground"> · {preview.packVersionId}</span>}
          </p>
          <p className="text-xs text-muted-foreground">
            {preview.plan.actions.length} change{preview.plan.actions.length === 1 ? '' : 's'},
            {' '}{preview.plan.conflicts.length} to decide.
            {preview.filesNeedingDownload > 0 && ` ${preview.filesNeedingDownload} file${preview.filesNeedingDownload === 1 ? '' : 's'} to download`}
            {preview.downloadBytes > 0 && ` (${formatBytes(preview.downloadBytes)})`}.
          </p>

          {preview.unverified.length > 0 && (
            <p className="text-xs text-muted-foreground">
              {preview.unverified.length} mod{preview.unverified.length === 1 ? '' : 's'} could not be
              compared without downloading, so those are shown as changes even if they turn out to
              be identical.
            </p>
          )}

          {preview.plan.baseline_missing && (
            <p className="text-xs text-muted-foreground">
              This pack was installed before Agora started tracking its files, so it cannot tell your
              edits apart from the pack's originals. Every file below is that same one question.
            </p>
          )}

          {preview.plan.conflicts.length > 0 && (
            <div className="space-y-2">
              <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                Both changed — you decide
              </h4>
              {preview.plan.conflicts.map((conflict) => (
                <div key={conflict.key} className="rounded-lg border border-border p-2">
                  <p className="text-sm">{conflict.logical_path}</p>
                  <p className="text-xs text-muted-foreground">{conflict.message}</p>
                  <div className="mt-2 flex gap-2">
                    {(['keep_ours', 'take_theirs'] as const).map((choice) => (
                      <button
                        key={choice}
                        type="button"
                        onClick={() => setResolutions((prev) => ({ ...prev, [conflict.key]: choice }))}
                        disabled={busy}
                        data-active={resolutions[conflict.key] === choice}
                        className="rounded border border-input px-2 py-1 text-xs hover:bg-accent data-[active=true]:bg-primary data-[active=true]:text-primary-foreground disabled:opacity-50"
                      >
                        {choice === 'keep_ours' ? 'Keep mine' : "Take the pack's"}
                      </button>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          )}

          <button
            type="button"
            onClick={() => void apply()}
            disabled={busy || locked || unresolved.length > 0}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
            title={unresolved.length > 0 ? `${unresolved.length} still to decide` : undefined}
          >
            {busy ? 'Updating…' : unresolved.length > 0 ? `${unresolved.length} still to decide` : 'Update'}
          </button>
        </div>
      )}
    </section>
  );
}
