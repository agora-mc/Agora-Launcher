/**
 * The Library's content store: the archives and folders the user has added, each kept once.
 * Add, check, and remove. The store is shared by every game; the core records a game only on
 * curated catalog items, so the section shows the items for its game plus those that name none,
 * and a switch shows the rest. A record the store cannot read is listed so it can be removed.
 */

import { useCallback, useEffect, useState } from 'react';
import { Button } from '@/components/ui/button';
import { useConfirm } from '@/components/ui/confirm';
import { isGamesError } from '../shared/types';
import {
  addToStore,
  formatBytes,
  itemsForGame,
  listStore,
  pickArchive,
  pickFolder,
  removeStored,
  verifyStored,
  type DamagedStoreItem,
  type StoreItem,
  type VerifyReport,
} from './api';

export interface LibraryStoreSectionProps {
  /** The game the Library tab is for. */
  gameId: string;
}

function errorMessage(error: unknown): string {
  if (isGamesError(error)) return error.message;
  if (error instanceof Error) return error.message;
  return 'Something went wrong. Try again.';
}

function fileCount(count: number): string {
  return `${count} file${count === 1 ? '' : 's'}`;
}

export function LibraryStoreSection({ gameId }: LibraryStoreSectionProps) {
  const { confirm } = useConfirm();
  const [items, setItems] = useState<StoreItem[] | null>(null);
  const [damaged, setDamaged] = useState<DamagedStoreItem[]>([]);
  const [showAll, setShowAll] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [reports, setReports] = useState<Record<string, VerifyReport>>({});

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const list = await listStore();
      setItems(list.items);
      setDamaged(list.damaged);
    } catch (error) {
      // Not "empty": the list did not come back, so the section says so and offers a retry.
      setItems(null);
      setDamaged([]);
      setLoadError(errorMessage(error));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const shown = items === null ? [] : itemsForGame(items, gameId, showAll);

  const addFrom = async (pick: () => Promise<string | null>, what: string) => {
    setNotice(null);
    setActionError(null);
    setBusy('add');
    try {
      const path = await pick();
      if (path === null) return;
      const added = await addToStore(path, null);
      setNotice(
        added.status === 'added'
          ? `Added ${added.item.name} (${fileCount(added.item.fileCount)}).`
          : `${added.item.name} was already in your library, so nothing new was stored.`,
      );
      await load();
    } catch (error) {
      setActionError(`Could not add the ${what}: ${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const check = async (item: StoreItem, full: boolean) => {
    setNotice(null);
    setActionError(null);
    setBusy(item.itemId);
    try {
      const report = await verifyStored(item.itemId, full);
      setReports((current) => ({ ...current, [item.itemId]: report }));
    } catch (error) {
      setActionError(`Could not check ${item.name}: ${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const remove = async (item: { itemId: string; name: string }) => {
    const confirmed = await confirm({
      title: `Remove ${item.name} from your library?`,
      body: 'Its files are deleted from Agora’s store. This cannot be undone.',
      confirmLabel: 'Remove',
      tone: 'danger',
    });
    if (!confirmed) return;
    setNotice(null);
    setActionError(null);
    setBusy(item.itemId);
    try {
      await removeStored(item.itemId);
      setNotice(`Removed ${item.name}.`);
      setReports((current) => {
        const next = { ...current };
        delete next[item.itemId];
        return next;
      });
      await load();
    } catch (error) {
      // The core refuses while an instance uses the item, and its message names the instances.
      setActionError(`Could not remove ${item.name}: ${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  return (
    <section aria-labelledby="library-store-heading" className="space-y-4" data-testid="library-store">
      <div>
        <h2 id="library-store-heading" className="text-base font-semibold">Your stored mods</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Archives and folders you have added. Each one is stored once, and every instance that uses it
          shares that copy.
        </p>
      </div>

      <div className="flex flex-wrap gap-2">
        <Button variant="outline" onClick={() => void addFrom(pickArchive, 'archive')} disabled={busy !== null}>
          Add an archive…
        </Button>
        <Button variant="outline" onClick={() => void addFrom(pickFolder, 'folder')} disabled={busy !== null}>
          Add a folder…
        </Button>
      </div>

      {notice && <p role="status" className="text-sm text-muted-foreground">{notice}</p>}
      {actionError && (
        <p role="alert" className="rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm">
          {actionError}
        </p>
      )}

      {loadError && (
        <div role="alert" className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm">
          <span>Your library could not be read: {loadError}</span>
          <Button variant="outline" size="sm" onClick={() => void load()}>Try again</Button>
        </div>
      )}

      {!loadError && items === null && <p className="text-sm text-muted-foreground">Reading your library…</p>}

      {items !== null && items.length > 0 && (
        <label className="flex items-center gap-2 text-sm text-muted-foreground">
          <input type="checkbox" checked={showAll} onChange={(event) => setShowAll(event.target.checked)} />
          Show mods for every game
        </label>
      )}

      {items !== null && items.length === 0 && damaged.length === 0 && (
        <p className="rounded-xl border border-dashed border-border p-4 text-sm text-muted-foreground">
          Your library is empty. Add an archive or a folder you downloaded, and it will show up here, ready to add to
          any instance.
        </p>
      )}

      {items !== null && items.length > 0 && shown.length === 0 && (
        <p className="rounded-xl border border-dashed border-border p-4 text-sm text-muted-foreground" data-testid="library-filtered-out">
          None of the {items.length} stored mod{items.length === 1 ? ' is' : 's are'} for this game. Turn on &ldquo;Show
          mods for every game&rdquo; to see {items.length === 1 ? 'it' : 'them'}.
        </p>
      )}

      {damaged.length > 0 && (
        <ul className="space-y-2" aria-label="Damaged records">
          {damaged.map((record) => (
            <li
              key={record.itemId}
              className="rounded-xl border border-destructive/50 bg-destructive/10 p-4"
              data-testid="library-damaged"
            >
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0 space-y-0.5">
                  <p className="font-medium">A stored record cannot be read</p>
                  <p className="text-sm">{record.message}</p>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => void remove({ itemId: record.itemId, name: record.itemId })}
                  disabled={busy !== null}
                >
                  Remove
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {items !== null && shown.length > 0 && (
        <ul className="space-y-2">
          {shown.map((item) => {
            const report = reports[item.itemId];
            return (
              <li key={item.itemId} className="rounded-xl border border-border bg-card p-4" data-testid="library-item">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 space-y-0.5">
                    <p className="truncate font-medium">{item.name}</p>
                    <p className="text-sm text-muted-foreground">{item.sourceLabel}</p>
                    <p className="text-xs text-muted-foreground">
                      {fileCount(item.fileCount)}, {formatBytes(item.totalSize)}
                    </p>
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <Button size="sm" variant="outline" onClick={() => void check(item, false)} disabled={busy !== null}>
                      Check
                    </Button>
                    <Button size="sm" variant="outline" onClick={() => void check(item, true)} disabled={busy !== null}>
                      Check everything
                    </Button>
                    <Button size="sm" variant="outline" onClick={() => void remove(item)} disabled={busy !== null}>
                      Remove
                    </Button>
                  </div>
                </div>
                {report && (
                  <div className="mt-3 text-sm" aria-live="polite">
                    {report.problems.length === 0 ? (
                      <p>Checked {fileCount(report.checked)}. Nothing has changed.</p>
                    ) : (
                      <div className="space-y-1">
                        <p className="font-medium">
                          {report.problems.length} problem{report.problems.length === 1 ? '' : 's'} found:
                        </p>
                        <ul className="list-disc space-y-1 pl-5">
                          {report.problems.map((problem) => (
                            <li key={`${problem.kind}:${problem.path}`}>
                              <span className="font-mono text-xs">{problem.path || '(whole item)'}</span>: {problem.message}
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
