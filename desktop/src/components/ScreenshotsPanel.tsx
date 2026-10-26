import { useCallback, useEffect, useState } from 'react';
import {
  deleteScreenshot,
  formatError,
  listScreenshots,
  readScreenshot,
  renameScreenshot,
  revealPath,
  type ScreenshotInfo,
} from '@/lib/tauri';
import { useConfirm } from '@/components/ui/confirm';
import { formatDateTime } from '@/lib/utils';

/** Screenshots are loaded this many at a time; each is a full image. */
const PAGE = 24;

/** Browse, view, copy, rename and delete an instance's screenshots. */
export function ScreenshotsPanel({
  instanceId,
  onStatus,
  onError,
}: {
  instanceId: string;
  onStatus: (message: string) => void;
  onError: (message: string) => void;
}) {
  const { confirm, prompt } = useConfirm();
  const [shots, setShots] = useState<ScreenshotInfo[] | null>(null);
  const [shown, setShown] = useState(PAGE);
  const [images, setImages] = useState<Record<string, string>>({});
  const [open, setOpen] = useState<ScreenshotInfo | null>(null);

  const reload = useCallback(async () => {
    try {
      const list = await listScreenshots(instanceId);
      setShots(Array.isArray(list) ? list : []);
    } catch (e) {
      setShots([]);
      onError(formatError(e));
    }
  }, [instanceId, onError]);

  useEffect(() => { void reload(); }, [reload]);

  // Load the visible page's images one by one, so the first ones appear quickly.
  useEffect(() => {
    if (!shots) return;
    let cancelled = false;
    void (async () => {
      for (const shot of shots.slice(0, shown)) {
        if (cancelled) return;
        if (images[shot.file]) continue;
        try {
          const url = await readScreenshot(instanceId, shot.file);
          if (!cancelled && typeof url === 'string') setImages((current) => ({ ...current, [shot.file]: url }));
        } catch {
          // A file that vanished or is too large just shows no thumbnail.
        }
      }
    })();
    return () => { cancelled = true; };
    // `images` is read, not watched: loading must not restart on each image.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shots, shown, instanceId]);

  const copyImage = async (shot: ScreenshotInfo) => {
    try {
      const url = images[shot.file] ?? await readScreenshot(instanceId, shot.file);
      // Decoded here rather than fetched: the page's connect-src allows no data: URLs.
      const [header, encoded] = url.split(',', 2);
      const binary = atob(encoded ?? '');
      const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
      const blob = new Blob([bytes], { type: header.slice(5, header.indexOf(';')) });
      // Clipboard images must be PNG; JPEG screenshots are copied as their path.
      if (blob.type === 'image/png' && typeof ClipboardItem !== 'undefined') {
        await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]);
        onStatus('Screenshot copied.');
      } else {
        await navigator.clipboard.writeText(shot.path);
        onStatus('Copied the file path.');
      }
    } catch (e) {
      onError(formatError(e));
    }
  };

  const rename = async (shot: ScreenshotInfo) => {
    const name = await prompt({ title: 'Rename screenshot', initialValue: shot.file.replace(/\.[^.]+$/, ''), confirmLabel: 'Rename' });
    if (!name?.trim()) return;
    try {
      const renamed = await renameScreenshot(instanceId, shot.file, name);
      setOpen(null);
      onStatus(`Renamed to ${renamed}.`);
      await reload();
    } catch (e) {
      onError(formatError(e));
    }
  };

  const remove = async (shot: ScreenshotInfo) => {
    const ok = await confirm({ title: `Delete ${shot.file}?`, body: 'It moves to the system recycle bin.', confirmLabel: 'Delete', tone: 'danger' });
    if (!ok) return;
    try {
      await deleteScreenshot(instanceId, shot.file);
      setOpen(null);
      await reload();
    } catch (e) {
      onError(formatError(e));
    }
  };

  const actions = (shot: ScreenshotInfo) => (
    <div className="flex flex-wrap gap-1.5">
      <button type="button" className="rounded-md border border-input bg-background px-2 py-1 text-xs hover:bg-accent" onClick={() => void copyImage(shot)}>Copy</button>
      <button type="button" className="rounded-md border border-input bg-background px-2 py-1 text-xs hover:bg-accent" onClick={() => void revealPath(shot.path)}>Show in folder</button>
      <button type="button" className="rounded-md border border-input bg-background px-2 py-1 text-xs hover:bg-accent" onClick={() => void rename(shot)}>Rename</button>
      <button type="button" className="rounded-md border border-input bg-background px-2 py-1 text-xs text-destructive hover:bg-accent" onClick={() => void remove(shot)}>Delete</button>
    </div>
  );

  if (shots === null) return <p className="text-sm text-muted-foreground">Loading screenshots…</p>;

  return (
    <section className="space-y-3 p-1" aria-label="Screenshots">
      {shots.length === 0 ? (
        <p className="rounded-lg border border-dashed border-border p-4 text-sm text-muted-foreground">
          No screenshots yet. Press F2 in game to take one.
        </p>
      ) : (
        <>
          <p className="text-xs text-muted-foreground">{shots.length} screenshot{shots.length === 1 ? '' : 's'}, newest first.</p>
          <ul className="grid grid-cols-[repeat(auto-fill,minmax(12rem,1fr))] gap-3">
            {shots.slice(0, shown).map((shot) => (
              <li key={shot.file} className="overflow-hidden rounded-lg border border-border bg-card" data-testid="screenshot">
                <button type="button" className="block w-full" onClick={() => setOpen(shot)} aria-label={`Open ${shot.file}`}>
                  {images[shot.file] ? (
                    <img src={images[shot.file]} alt="" className="aspect-video w-full object-cover" />
                  ) : (
                    <div className="aspect-video w-full animate-pulse bg-muted" />
                  )}
                </button>
                <div className="px-2 py-1.5 text-xs">
                  <p className="truncate font-medium" title={shot.file}>{shot.file}</p>
                  <p className="text-muted-foreground">{formatDateTime(shot.taken)}</p>
                </div>
              </li>
            ))}
          </ul>
          {shown < shots.length && (
            <button type="button" className="rounded-lg border border-input px-3 py-1.5 text-sm hover:bg-accent" onClick={() => setShown((n) => n + PAGE)}>
              Show more ({shots.length - shown} left)
            </button>
          )}
        </>
      )}

      {open && (
        <div
          role="dialog"
          aria-modal="true"
          aria-label={open.file}
          className="fixed inset-0 z-50 flex flex-col items-center justify-center gap-3 bg-black/80 p-6"
          onKeyDown={(event) => { if (event.key === 'Escape') setOpen(null); }}
          // controller-exempt: backdrop, not a control. The focused Close button
          // and Escape close the viewer for keyboard and controller users.
          onClick={(event) => { if (event.target === event.currentTarget) setOpen(null); }}
        >
          <div className="flex max-h-full max-w-full flex-col items-center gap-3">
            {images[open.file] && <img src={images[open.file]} alt={open.file} className="max-h-[80vh] max-w-full rounded-md object-contain" />}
            <div className="flex flex-wrap items-center gap-3 rounded-lg bg-card p-2">
              <span className="text-sm font-medium">{open.file}</span>
              {actions(open)}
              <button type="button" className="rounded-md px-2 py-1 text-xs hover:bg-accent" onClick={() => setOpen(null)} autoFocus>Close</button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
