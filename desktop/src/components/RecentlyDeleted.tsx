import { useEffect, useState } from 'react';
import { ArchiveRestore, Trash2 } from 'lucide-react';
import {
  formatError,
  listRecentlyDeleted,
  purgeDeletedInstance,
  restoreDeletedInstance,
  type RecentlyDeletedEntry,
} from '../lib/tauri';
import { useConfirm } from '@/components/ui/confirm';
import { importSourceLabel, isFileImportSource } from '../lib/importDisplay';

/** How long Agora keeps a deleted instance before moving it to the OS trash. */
export const RECENTLY_DELETED_DAYS = 30;

function launcherName(source: string | null | undefined): string | null {
  if (!source) return null;
  if (isFileImportSource(source)) return importSourceLabel(source);
  if (source === 'curse_forge' || source === 'curseforge') return 'CurseForge';
  if (source === 'prism') return 'Prism Launcher';
  return 'Modrinth App';
}

/** Say exactly what Delete does: Agora's copy only, restorable for 30 days. */
export function deleteDialogBody(instance: { name: string; import_source?: string | null }): string {
  const launcher = launcherName(instance.import_source);
  const untouched = isFileImportSource(instance.import_source)
    ? ` The pack file ${launcher} isn't touched.`
    : launcher ? ` Your ${launcher} files aren't touched.` : '';
  return `Removes Agora's copy of ${instance.name}.${untouched} You can restore it from Recently deleted for ${RECENTLY_DELETED_DAYS} days.`;
}

function formatSize(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  if (bytes >= 1024 ** 2) return `${Math.round(bytes / 1024 ** 2)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

function formatDay(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '';
  return new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' }).format(date);
}

/**
 * Instances deleted in the last 30 days, with Restore and Delete now.
 * Renders nothing while the list is empty. `refreshKey` changes when the
 * instance list does, so a fresh delete shows up without a manual reload.
 */
export function RecentlyDeleted({
  refreshKey,
  onRestored,
}: {
  refreshKey: unknown;
  onRestored: () => void;
}) {
  const { confirm } = useConfirm();
  const [entries, setEntries] = useState<RecentlyDeletedEntry[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const reload = async () => {
    try {
      const rows = await listRecentlyDeleted();
      setEntries(Array.isArray(rows) ? rows : []);
    } catch (cause) {
      setError(formatError(cause));
    }
  };

  useEffect(() => {
    void reload();
  }, [refreshKey]);

  const restore = async (entry: RecentlyDeletedEntry) => {
    setBusy(entry.entry_id);
    setError(null);
    setNotice(null);
    try {
      const outcome = await restoreDeletedInstance(entry.entry_id);
      setNotice(
        outcome.renamed
          ? `Restored ${entry.name} as "${entry.name} (restored)" because "${outcome.original_instance_id}" is in use by another instance.`
          : `Restored ${entry.name}.`,
      );
      onRestored();
      await reload();
    } catch (cause) {
      setError(formatError(cause));
    } finally {
      setBusy(null);
    }
  };

  const deleteNow = async (entry: RecentlyDeletedEntry) => {
    if (!await confirm({
      title: `Delete "${entry.name}" now?`,
      body: "This sends Agora's copy to your Recycle Bin straight away. After that you can only get it back from there, not from Agora.",
      confirmLabel: 'Delete now',
      tone: 'danger',
    })) return;
    setBusy(entry.entry_id);
    setError(null);
    setNotice(null);
    try {
      await purgeDeletedInstance(entry.entry_id);
      await reload();
    } catch (cause) {
      setError(formatError(cause));
    } finally {
      setBusy(null);
    }
  };

  if (entries.length === 0 && !error && !notice) return null;

  return (
    <section aria-labelledby="recently-deleted-heading" className="space-y-3 rounded-xl border border-border bg-card p-4">
      <div>
        <h2 id="recently-deleted-heading" className="text-base font-semibold">Recently deleted</h2>
        <p className="text-sm text-muted-foreground">
          Deleted instances stay here for {RECENTLY_DELETED_DAYS} days, then move to your Recycle Bin.
        </p>
      </div>
      {notice && <p role="status" className="text-sm text-foreground">{notice}</p>}
      {error && (
        <p role="alert" className="rounded-lg bg-destructive p-2 text-sm text-destructive-foreground">{error}</p>
      )}
      <ul className="divide-y divide-border">
        {entries.map((entry) => (
          <li key={entry.entry_id} className="flex flex-wrap items-center justify-between gap-3 py-2">
            <div className="min-w-0">
              <p className="wrap-break-word font-medium">{entry.name}</p>
              <p className="text-xs text-muted-foreground">
                Deleted {formatDay(entry.deleted_at)} · {formatSize(entry.size_bytes)} · removed for good after {formatDay(entry.expires_at)}
              </p>
            </div>
            <div className="flex gap-2">
              <button
                type="button"
                disabled={busy !== null}
                onClick={() => void restore(entry)}
                className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-card px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50"
              >
                <ArchiveRestore className="h-4 w-4" aria-hidden="true" />
                Restore
              </button>
              <button
                type="button"
                disabled={busy !== null}
                onClick={() => void deleteNow(entry)}
                className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-card px-3 py-1.5 text-sm font-medium text-destructive hover:bg-accent disabled:opacity-50"
              >
                <Trash2 className="h-4 w-4" aria-hidden="true" />
                Delete now
              </button>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
