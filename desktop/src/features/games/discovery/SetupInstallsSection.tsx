/**
 * Setup › Installs: every game Agora found on this computer, each copy with its store, version and
 * folder, the warnings from the scan, and the game folders the user added by hand (MASTER_SPEC
 * §26.3). Rescan runs both lookups again.
 *
 * A failed lookup shows its error and no list, so a broken scan is never read as "no games found".
 */

import { useCallback, useEffect, useState } from 'react';
import { FolderPlus, RefreshCw, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useConfirm } from '@/components/ui/confirm';
import { pickDirectory } from '@/lib/tauri';
import { isGamesError, type InstallSummary } from '../shared/types';
import {
  addGameFolder,
  discoverGames,
  listInstalledGames,
  removeGameFolder,
  type GameFolderScan,
  type GameListing,
  type GamesList,
  type ListedGameFolder,
  type ScanWarning,
} from './api';

interface SetupInstallsSectionProps {
  /** The game the Setup tab was opened for. Installs of every game are listed here. */
  gameId: string;
}

const STORE_NAMES: Record<string, string> = {
  steam: 'Steam',
  gog: 'GOG',
  epic: 'Epic Games Store',
  'microsoft-store': 'Microsoft Store',
  direct: 'a folder you added',
};

function storeName(store: string): string {
  return STORE_NAMES[store] ?? store;
}

/** The message to show for a failure: the core's own sentence when there is one. */
function messageOf(error: unknown): string {
  if (isGamesError(error)) return error.message;
  if (error instanceof Error) return error.message;
  return 'Something went wrong. Try again.';
}

/** Why an install's version or runtime is unknown, as sentences for a person. */
function installWarnings(install: InstallSummary): string[] {
  if (install.runtime.status === 'identified') return [];
  return install.runtime.reasons.length > 0
    ? install.runtime.reasons
    : ['Agora could not tell which game version this is.'];
}

function installVersion(install: InstallSummary): string {
  if (install.runtime.status === 'identified') return `Version ${install.runtime.runtime.version}`;
  return install.storeVersion ? `Version ${install.storeVersion}` : 'Version not read';
}

export function SetupInstallsSection(_props: SetupInstallsSectionProps) {
  const { confirm } = useConfirm();
  const [games, setGames] = useState<GamesList | null>(null);
  const [scan, setScan] = useState<GameFolderScan | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [listed, found] = await Promise.all([listInstalledGames(), discoverGames()]);
      setGames(listed);
      setScan(found);
      setError(null);
    } catch (caught) {
      setGames(null);
      setScan(null);
      setError(messageOf(caught));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const addFolder = async () => {
    setNotice(null);
    let picked: string | null;
    try {
      picked = await pickDirectory('Choose the folder the game is installed in');
    } catch (caught) {
      setError(messageOf(caught));
      return;
    }
    if (!picked) return;
    setBusy(true);
    setError(null);
    try {
      const added = await addGameFolder(picked);
      setNotice(`Added ${added.path} as ${added.gameName}.`);
      await load();
    } catch (caught) {
      setError(messageOf(caught));
    } finally {
      setBusy(false);
    }
  };

  const removeFolder = async (folder: ListedGameFolder) => {
    const ok = await confirm({
      title: 'Stop searching this folder?',
      body: `Agora will forget ${folder.path}. Nothing in the folder is deleted or changed.`,
      confirmLabel: 'Stop searching',
      tone: 'danger',
    });
    if (!ok) return;
    setNotice(null);
    setBusy(true);
    setError(null);
    try {
      const removed = await removeGameFolder(folder.path);
      setNotice(`Stopped searching ${removed}. Its files were not touched.`);
      await load();
    } catch (caught) {
      setError(messageOf(caught));
    } finally {
      setBusy(false);
    }
  };

  const rescan = async () => {
    setNotice(null);
    setBusy(true);
    await load();
    setBusy(false);
  };

  const found: GameListing[] = games?.games ?? [];
  const folders = scan?.folders ?? [];
  const warnings: ScanWarning[] = scan?.warnings ?? [];

  return (
    <section
      aria-labelledby="setup-installs-heading"
      className="space-y-5 rounded-xl border border-border bg-card/60 p-6"
      data-testid="setup-installs"
    >
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 id="setup-installs-heading" className="text-base font-semibold text-foreground">Installs</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Copies of games Agora found on this computer, from the stores it checks and from folders you added.
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="outline" size="sm" onClick={rescan} disabled={busy || loading}>
            <RefreshCw aria-hidden="true" />
            Rescan
          </Button>
          <Button type="button" size="sm" onClick={addFolder} disabled={busy || loading}>
            <FolderPlus aria-hidden="true" />
            Add a game folder…
          </Button>
        </div>
      </header>

      {notice && (
        <p role="status" className="rounded-md border border-border bg-muted/40 px-3 py-2 text-sm text-foreground">
          {notice}
        </p>
      )}

      {error && (
        <div role="alert" className="space-y-2 rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm">
          <p className="text-foreground">{error}</p>
          <Button type="button" variant="outline" size="sm" onClick={() => void load()}>
            Try again
          </Button>
        </div>
      )}

      {loading && games === null && !error && (
        <p className="text-sm text-muted-foreground">Looking for installed games…</p>
      )}

      {games && found.length === 0 && (
        <div data-testid="installs-empty" className="rounded-md border border-dashed border-border p-4 text-sm text-muted-foreground">
          <p className="font-medium text-foreground">Agora did not find this game on this computer.</p>
          <p className="mt-1">
            Stores are checked automatically. If the game is in a folder Agora does not search, such as
            another drive or a portable copy, use <span className="text-foreground">Add a game folder…</span> and
            pick the folder the game is in.
          </p>
        </div>
      )}

      {found.map((listing) => (
        <div key={listing.gameId} className="space-y-2" data-testid="installs-game" data-game-id={listing.gameId}>
          <h3 className="text-sm font-semibold text-foreground">
            {listing.name}
            <span className="ml-2 font-normal text-muted-foreground">
              {listing.installs.length === 1 ? '1 install' : `${listing.installs.length} installs`}
            </span>
          </h3>
          <ul className="space-y-2">
            {listing.installs.map((install) => {
              const issues = installWarnings(install);
              return (
                <li
                  key={install.installId}
                  className="rounded-lg border border-border bg-background/60 p-3 text-sm"
                  data-testid="install-card"
                  data-install-id={install.installId}
                >
                  <p className="font-medium text-foreground">{install.name}</p>
                  <p className="text-muted-foreground">
                    {storeName(install.store)} · {installVersion(install)}
                    {install.addOnCount > 0 && ` · ${install.addOnCount} add-on${install.addOnCount === 1 ? '' : 's'}`}
                  </p>
                  <p className="mt-1 break-all font-mono text-xs text-muted-foreground" title={install.location}>
                    {install.location}
                  </p>
                  {issues.map((issue) => (
                    <p key={issue} className="mt-1 text-xs text-amber-600 dark:text-amber-400">
                      Warning: {issue}
                    </p>
                  ))}
                </li>
              );
            })}
          </ul>
        </div>
      ))}

      {scan && warnings.length > 0 && (
        <div data-testid="scan-warnings" className="space-y-1 rounded-md border border-amber-500/40 p-3 text-sm">
          <p className="font-medium text-foreground">Agora could not read everything it looked at:</p>
          <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
            {warnings.map((warning, index) => (
              <li key={`${warning.store}-${index}`}>
                {warning.store === 'direct' ? '' : `${storeName(warning.store)}: `}
                {warning.message}
              </li>
            ))}
          </ul>
        </div>
      )}

      {scan && (
        <div className="space-y-2" data-testid="setup-folders">
          <h3 className="text-sm font-semibold text-foreground">Folders you added</h3>
          {folders.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              None yet. A copy that no store lists can still be added, with Add a game folder….
            </p>
          ) : (
            <ul className="space-y-2">
              {folders.map((folder) => (
                <li
                  key={folder.path}
                  className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-border bg-background/60 p-3 text-sm"
                  data-testid="setup-folder-row"
                >
                  <div className="min-w-0">
                    <p className="break-all font-mono text-xs text-foreground">{folder.path}</p>
                    <p className="text-xs text-muted-foreground">
                      {folder.present ? 'Found on this computer' : 'Not there now, so it is not searched'}
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    aria-label={`Stop searching ${folder.path}`}
                    onClick={() => void removeFolder(folder)}
                    disabled={busy}
                  >
                    <Trash2 aria-hidden="true" />
                    Remove
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
