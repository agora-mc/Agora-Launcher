import { useEffect, useState } from 'react';
import {
  createDesktopShortcut,
  formatError,
  getSharedScreenshotStatus,
  linkSharedScreenshots,
  unlinkSharedScreenshots,
  type SharedScreenshotStatus,
} from '@/lib/tauri';

/**
 * Desktop integration: a shortcut straight to this instance, and a screenshot
 * folder shared with every other instance that opts in.
 */
export function InstanceIntegrationPanel({
  instanceId,
  displayName,
}: {
  instanceId: string;
  displayName: string;
}) {
  const [shots, setShots] = useState<SharedScreenshotStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = () => {
    getSharedScreenshotStatus(instanceId)
      .then(setShots)
      .catch((e) => setError(formatError(e)));
  };
  useEffect(refresh, [instanceId]);

  const run = async (action: () => Promise<string | void>) => {
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      const message = await action();
      if (typeof message === 'string') setStatus(message);
      refresh();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="rounded-xl border border-border bg-card p-4 space-y-4">
      <h3 className="font-semibold text-sm">Desktop integration</h3>

      {error && <p className="text-sm text-destructive">{error}</p>}
      {status && <p className="text-sm text-primary">{status}</p>}

      <div className="space-y-1">
        <button
          type="button"
          onClick={() => void run(async () => {
            const path = await createDesktopShortcut(instanceId, displayName);
            return `Shortcut created at ${path}`;
          })}
          disabled={busy}
          className="rounded-lg border border-input bg-background px-4 py-2 text-sm font-medium hover:bg-accent disabled:opacity-50"
        >
          Create desktop shortcut
        </button>
        <p className="text-xs text-muted-foreground">
          Launches this instance directly. If Agora is already open, the shortcut tells it to launch
          rather than opening a second window.
        </p>
      </div>

      <div className="space-y-1 border-t border-border pt-3">
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => void run(() => shots?.linked
              ? unlinkSharedScreenshots(instanceId).then(() => 'Screenshots are no longer shared.')
              : linkSharedScreenshots(instanceId))}
            disabled={busy || !shots}
            className="rounded-lg border border-input bg-background px-4 py-2 text-sm font-medium hover:bg-accent disabled:opacity-50"
          >
            {shots?.linked ? 'Stop sharing screenshots' : 'Share screenshots with other instances'}
          </button>
          {shots?.linked && (
            <span className="text-xs text-muted-foreground">Linked to {shots.target ?? shots.shared_root}</span>
          )}
        </div>
        <p className="text-xs text-muted-foreground">
          Minecraft always writes screenshots inside the instance folder, so sharing works by
          linking that folder to one shared location. Screenshots already here are moved across,
          never deleted; turning sharing off leaves the shared ones alone.
        </p>
      </div>
    </section>
  );
}
