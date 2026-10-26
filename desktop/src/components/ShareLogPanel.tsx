import { useEffect, useState } from 'react';
import {
  formatError,
  listShareableLogs,
  openExternalUrl,
  previewLogShare,
  shareLog,
  type LogPreview,
  type ShareableLog,
} from '@/lib/tauri';

/**
 * Share a game log or crash report as a mclo.gs link. The user sees exactly
 * what will be uploaded before anything leaves the machine.
 */
export function ShareLogPanel({ instanceId }: { instanceId: string }) {
  const [logs, setLogs] = useState<ShareableLog[] | null>(null);
  const [path, setPath] = useState('');
  const [preview, setPreview] = useState<LogPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [link, setLink] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listShareableLogs(instanceId)
      .then((found) => {
        const list = Array.isArray(found) ? found : [];
        setLogs(list);
        setPath(list[0]?.path ?? '');
      })
      .catch((e) => { setLogs([]); setError(formatError(e)); });
  }, [instanceId]);

  const showPreview = async () => {
    setError(null);
    setLink(null);
    setBusy(true);
    try {
      setPreview(await previewLogShare(instanceId, path));
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const upload = async () => {
    setError(null);
    setBusy(true);
    try {
      const shared = await shareLog(instanceId, path);
      setLink(shared.url);
      setPreview(null);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  if (logs === null) return null;

  return (
    <section className="rounded-xl border border-border bg-card p-4 space-y-3" aria-labelledby="share-log-heading">
      <h3 id="share-log-heading" className="font-semibold text-sm">Share a log</h3>
      <p className="text-xs text-muted-foreground">
        Get a mclo.gs link to paste into a support channel. You see the exact text first; your user folder name and anything
        that looks like a token are removed. Anyone with the link can read it.
      </p>
      {logs.length === 0 ? (
        <p className="text-sm text-muted-foreground">No logs yet. Launch the instance once.</p>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <select
            aria-label="Log to share"
            value={path}
            onChange={(e) => { setPath(e.target.value); setPreview(null); setLink(null); }}
            className="rounded-lg border border-input bg-background px-3 py-1.5 text-sm"
          >
            {logs.map((log) => <option key={log.path} value={log.path}>{log.label}</option>)}
          </select>
          <button type="button" disabled={busy || !path} onClick={() => void showPreview()} className="rounded-lg border border-input px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50">
            Review what will be shared
          </button>
        </div>
      )}
      {preview && (
        <div className="space-y-2">
          <textarea
            aria-label="Text that will be uploaded"
            readOnly
            value={preview.text}
            rows={10}
            className="w-full rounded-lg border border-input bg-background px-3 py-2 font-mono text-xs"
          />
          {preview.dropped_lines > 0 && (
            <p className="text-xs text-muted-foreground">The oldest {preview.dropped_lines} lines are left off to fit mclo.gs&apos;s limit.</p>
          )}
          <button type="button" disabled={busy} onClick={() => void upload()} className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50">
            {busy ? 'Uploading…' : 'Upload to mclo.gs'}
          </button>
        </div>
      )}
      {link && (
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <span>Shared:</span>
          <code className="rounded bg-muted px-2 py-0.5">{link}</code>
          <button type="button" onClick={() => void navigator.clipboard?.writeText(link)} className="rounded-md border border-input px-2 py-1 text-xs hover:bg-accent">Copy link</button>
          <button type="button" onClick={() => void openExternalUrl(link)} className="rounded-md border border-input px-2 py-1 text-xs hover:bg-accent">Open</button>
        </div>
      )}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </section>
  );
}
