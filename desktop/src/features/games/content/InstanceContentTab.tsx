/**
 * An instance's Content tab: the content it has, in load order, with where each item came from.
 * The user turns items on and off, moves them, removes them, and adds items from the library.
 * An archive with a FOMOD installer opens the installer's questions before anything is added.
 */

import { useCallback, useEffect, useState } from 'react';
import { Button } from '@/components/ui/button';
import { useConfirm } from '@/components/ui/confirm';
import { FomodWizard } from '../fomod';
import { isGamesError } from '../shared/types';
import {
  addInstanceContent,
  listInstanceContent,
  listStore,
  moveInstanceContent,
  removeInstanceContent,
  setInstanceContentEnabled,
  type InstanceContentList,
  type InstanceContentRow,
  type InstanceAddOutcome,
  type StoreItem,
} from './api';

export interface InstanceContentTabProps {
  /** The game the instance belongs to. The instance's own record says which game it is. */
  gameId: string;
  instanceId: string;
}

function errorMessage(error: unknown): string {
  if (isGamesError(error)) return error.message;
  if (error instanceof Error) return error.message;
  return 'Something went wrong. Try again.';
}

function whereItGoes(row: InstanceContentRow): string {
  const mount = row.mountPath ? `the ${row.mountPath} folder` : 'the game folder';
  return row.sourcePath ? `Installs ${row.sourcePath} into ${mount}` : `Installs into ${mount}`;
}

function addedNotice(outcome: Extract<InstanceAddOutcome, { status: 'added' }>, name: string): string {
  if (outcome.thunderstore) {
    const pkg = outcome.thunderstore;
    const missing = pkg.missingDependencies.length > 0
      ? ` It needs ${pkg.missingDependencies.join(', ')}, which this instance does not have yet.`
      : '';
    return `Installed Thunderstore package ${pkg.packageId} ${pkg.version}. ${pkg.summary}${missing}`;
  }
  return outcome.placementReason
    ? `Added ${name}. ${outcome.placementReason}`
    : `Added ${name}.`;
}

export function InstanceContentTab({ instanceId }: InstanceContentTabProps) {
  const { confirm } = useConfirm();
  const [content, setContent] = useState<InstanceContentList | null>(null);
  const [contentError, setContentError] = useState<string | null>(null);
  const [store, setStore] = useState<StoreItem[] | null>(null);
  const [storeError, setStoreError] = useState<string | null>(null);
  const [pickedId, setPickedId] = useState('');
  const [notice, setNotice] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [wizardFor, setWizardFor] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setContentError(null);
    setStoreError(null);
    const [contentResult, storeResult] = await Promise.allSettled([
      listInstanceContent(instanceId),
      listStore(),
    ]);
    if (contentResult.status === 'fulfilled') {
      setContent(contentResult.value);
    } else {
      setContent(null);
      setContentError(errorMessage(contentResult.reason));
    }
    if (storeResult.status === 'fulfilled') {
      setStore(storeResult.value.items);
    } else {
      setStore(null);
      setStoreError(errorMessage(storeResult.reason));
    }
  }, [instanceId]);

  useEffect(() => {
    setNotice(null);
    setActionError(null);
    setPickedId('');
    void refresh();
  }, [refresh]);

  /** Run one change, then reread the tab so it shows what the core now holds. */
  const change = async (key: string, action: () => Promise<unknown>, failure: string) => {
    setNotice(null);
    setActionError(null);
    setBusy(key);
    try {
      await action();
      await refresh();
    } catch (error) {
      setActionError(`${failure} ${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const addSelected = async () => {
    const item = store?.find((candidate) => candidate.itemId === pickedId);
    if (!item) return;
    setNotice(null);
    setActionError(null);
    setBusy('add');
    try {
      const outcome = await addInstanceContent(instanceId, item.itemId);
      if (outcome.status === 'needs_installer') {
        setNotice(`${item.name} has an installer. Answer its questions to choose what gets added.`);
        setWizardFor(outcome.itemId);
      } else {
        setNotice(addedNotice(outcome, item.name));
        await refresh();
      }
      setPickedId('');
    } catch (error) {
      setActionError(`Could not add ${item.name}: ${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const removeRow = async (row: InstanceContentRow) => {
    const confirmed = await confirm({
      title: `Remove ${row.name} from this instance?`,
      body: 'It stays in your library, so you can add it again later.',
      confirmLabel: 'Remove',
      tone: 'danger',
    });
    if (!confirmed) return;
    await change(row.itemId, () => removeInstanceContent(instanceId, row.itemId), `Could not remove ${row.name}.`);
  };

  const rows = content?.items ?? [];
  const inInstance = new Set(rows.map((row) => row.itemId));
  const addable = (store ?? []).filter((item) => !inInstance.has(item.itemId));

  return (
    <section aria-labelledby="instance-content-heading" className="space-y-4" data-testid="instance-content">
      <div>
        <h2 id="instance-content-heading" className="text-base font-semibold">Content</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          What this instance uses, in load order. Lower rows load first, so a later row wins when two items change the
          same file.
        </p>
      </div>

      {notice && <p role="status" className="text-sm text-muted-foreground">{notice}</p>}
      {actionError && (
        <p role="alert" className="rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm">
          {actionError}
        </p>
      )}

      {contentError && (
        <div role="alert" className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm">
          <span>This instance’s content could not be read: {contentError}</span>
          <Button variant="outline" size="sm" onClick={() => void refresh()}>Try again</Button>
        </div>
      )}

      {!contentError && content === null && <p className="text-sm text-muted-foreground">Reading the content…</p>}

      {content !== null && rows.length === 0 && (
        <p className="rounded-xl border border-dashed border-border p-4 text-sm text-muted-foreground">
          This instance has no content yet. Add something from your library below. If your library is empty, add an
          archive or folder on the Library tab first.
        </p>
      )}

      {rows.length > 0 && (
        <ol className="space-y-2">
          {rows.map((row) => {
            return (
              <li key={row.layerId} className="rounded-xl border border-border bg-card p-4" data-testid="instance-content-row">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 space-y-0.5">
                    <p className="font-medium">
                      <span className="mr-2 tabular-nums text-muted-foreground">{row.position}.</span>
                      {row.name}
                      <span className={`ml-2 rounded px-1.5 py-0.5 text-xs ${row.enabled ? 'bg-primary/15' : 'bg-muted text-muted-foreground'}`}>
                        {row.enabled ? 'On' : 'Off'}
                      </span>
                    </p>
                    <p className="text-sm text-muted-foreground">{row.sourceLabel}</p>
                    <p className="text-xs text-muted-foreground">{whereItGoes(row)}</p>
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <Button
                      size="sm"
                      variant="outline"
                      aria-label={`Move ${row.name} up (loads later)`}
                      onClick={() => void change(row.itemId, () => moveInstanceContent(instanceId, row.itemId, row.position + 1), `Could not move ${row.name}.`)}
                      disabled={busy !== null || row.position === rows.length}
                    >
                      Move up
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      aria-label={`Move ${row.name} down (loads earlier)`}
                      onClick={() => void change(row.itemId, () => moveInstanceContent(instanceId, row.itemId, row.position - 1), `Could not move ${row.name}.`)}
                      disabled={busy !== null || row.position === 1}
                    >
                      Move down
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => void change(row.itemId, () => setInstanceContentEnabled(instanceId, row.itemId, !row.enabled), `Could not turn ${row.name} ${row.enabled ? 'off' : 'on'}.`)}
                      disabled={busy !== null}
                    >
                      {row.enabled ? 'Turn off' : 'Turn on'}
                    </Button>
                    <Button size="sm" variant="outline" onClick={() => void removeRow(row)} disabled={busy !== null}>
                      Remove
                    </Button>
                  </div>
                </div>
              </li>
            );
          })}
        </ol>
      )}

      <div className="space-y-2 rounded-xl border border-border p-4">
        <h3 className="text-sm font-medium">Add from your library</h3>
        {storeError && (
          <p role="alert" className="text-sm">Your library could not be read, so nothing can be added: {storeError}</p>
        )}
        {!storeError && store !== null && addable.length === 0 && (
          <p className="text-sm text-muted-foreground">
            {store.length === 0
              ? 'Your library is empty. Add an archive or folder on the Library tab first.'
              : 'Everything in your library is already in this instance.'}
          </p>
        )}
        {addable.length > 0 && (
          <div className="flex flex-wrap items-center gap-2">
            <label className="sr-only" htmlFor="instance-content-add">Choose an item to add</label>
            <select
              id="instance-content-add"
              className="h-10 min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm"
              value={pickedId}
              onChange={(event) => setPickedId(event.target.value)}
            >
              <option value="">Choose an item…</option>
              {addable.map((item) => (
                <option key={item.itemId} value={item.itemId}>{item.name}</option>
              ))}
            </select>
            <Button onClick={() => void addSelected()} disabled={pickedId === '' || busy !== null}>
              Add to instance
            </Button>
          </div>
        )}
      </div>

      {wizardFor && (
        <FomodWizard
          contentId={wizardFor}
          instanceId={instanceId}
          onClose={() => {
            setWizardFor(null);
            void refresh();
          }}
        />
      )}
    </section>
  );
}
