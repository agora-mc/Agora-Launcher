import { useEffect, useId, useState } from 'react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { listInstalledGames } from '../discovery/api';
import type { GameListing, InstallSummary } from '../shared/types';
import {
  createGameInstance,
  gamesErrorMessage,
  onCreateProgress,
  type BaseMode,
  type CreateProgress,
} from './api';
import { baseModeCopy, DEPLOYMENT_OPTIONS, progressText } from './instanceText';

interface CreateInstanceDialogProps {
  open: boolean;
  gameId: string;
  onOpenChange: (open: boolean) => void;
  /** Called with the new instance's id once it exists. */
  onCreated: (instanceId: string) => void;
}

type InstallsState =
  | { status: 'loading' }
  | { status: 'failed'; message: string }
  | { status: 'loaded'; game: GameListing | null };

const BASE_MODES: readonly BaseMode[] = ['linked', 'copied'];

/**
 * Creates an instance from an install of the game. The base is built from the install (or an
 * identical one is reused). The deployment choice is explained here and made on the instance's
 * own page, where it can be changed later.
 */
export function CreateInstanceDialog({ open, gameId, onOpenChange, onCreated }: CreateInstanceDialogProps) {
  const nameId = useId();
  const installId = useId();
  const [installs, setInstalls] = useState<InstallsState>({ status: 'loading' });
  const [name, setName] = useState('');
  const [chosenInstall, setChosenInstall] = useState('');
  const [baseMode, setBaseMode] = useState<BaseMode>('linked');
  const [creating, setCreating] = useState(false);
  const [progress, setProgress] = useState<CreateProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setInstalls({ status: 'loading' });
    setError(null);
    listInstalledGames()
      .then((list) => {
        const game = list.games.find((listing) => listing.gameId === gameId) ?? null;
        setInstalls({ status: 'loaded', game });
        setChosenInstall(game?.installs[0]?.installId ?? '');
      })
      .catch((failure: unknown) => setInstalls({ status: 'failed', message: gamesErrorMessage(failure) }));
  }, [open, gameId]);

  const available: InstallSummary[] = installs.status === 'loaded' ? (installs.game?.installs ?? []) : [];
  const gameName = installs.status === 'loaded' && installs.game ? installs.game.name : 'this game';
  const selected = available.find((install) => install.installId === chosenInstall) ?? null;
  const canCreate = !creating && name.trim() !== '' && selected !== null;

  const submit = async () => {
    if (!canCreate || selected === null) return;
    setCreating(true);
    setError(null);
    setProgress(null);
    const unlisten = await onCreateProgress(setProgress).catch(() => null);
    try {
      const created = await createGameInstance(gameId, selected.installId, name, baseMode);
      onCreated(created.instanceId);
    } catch (failure) {
      setError(`The instance was not created. ${gamesErrorMessage(failure)}`);
    } finally {
      unlisten?.();
      setCreating(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => { if (!creating) onOpenChange(next); }}>
      <DialogContent className="max-h-[90vh] max-w-2xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>New instance of {gameName}</DialogTitle>
          <DialogDescription>
            An instance is a separate setup of the game. Its mods and saves stay with it, and the game install is shared.
          </DialogDescription>
        </DialogHeader>

        {installs.status === 'loading' && <p role="status" className="text-sm text-muted-foreground">Looking for installs…</p>}

        {installs.status === 'failed' && (
          <p role="alert" className="text-sm text-destructive">
            Agora could not look for installs. {installs.message}
          </p>
        )}

        {installs.status === 'loaded' && available.length === 0 && (
          <p role="alert" className="rounded-lg border border-border p-3 text-sm">
            Agora has not found an install of {gameName} on this machine. Rescan the installs in Setup, then try again.
          </p>
        )}

        {installs.status === 'loaded' && available.length > 0 && (
          <form
            className="space-y-5"
            onSubmit={(event) => {
              event.preventDefault();
              void submit();
            }}
          >
            <div className="space-y-2">
              <Label htmlFor={nameId}>Name</Label>
              <Input
                id={nameId}
                value={name}
                placeholder={`For example, ${gameName} with my mods`}
                onChange={(event) => setName(event.target.value)}
                disabled={creating}
                autoComplete="off"
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor={installId}>Install to build from</Label>
              <select
                id={installId}
                className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                value={chosenInstall}
                onChange={(event) => setChosenInstall(event.target.value)}
                disabled={creating}
              >
                {available.map((install) => (
                  <option key={install.installId} value={install.installId}>
                    {install.name} · {install.store}{install.storeVersion ? ` ${install.storeVersion}` : ''}
                  </option>
                ))}
              </select>
              {selected !== null && (
                <p className="text-xs text-muted-foreground">{selected.location}</p>
              )}
            </div>

            <fieldset className="space-y-2" disabled={creating}>
              <legend className="text-sm font-medium">Base</legend>
              <p className="text-xs text-muted-foreground">
                The base is a pinned copy of the game files that instances share. Agora reuses one that already exists.
              </p>
              <div className="grid gap-2 sm:grid-cols-2">
                {BASE_MODES.map((mode) => {
                  const copy = baseModeCopy(mode);
                  const chosen = mode === baseMode;
                  return (
                    <button
                      key={mode}
                      type="button"
                      aria-pressed={chosen}
                      onClick={() => setBaseMode(mode)}
                      className={[
                        'rounded-lg border p-3 text-left text-sm transition-colors',
                        chosen ? 'border-primary bg-primary/10' : 'border-border hover:bg-accent',
                      ].join(' ')}
                    >
                      <span className="block font-semibold">{copy.label}</span>
                      <span className="mt-1 block text-xs text-muted-foreground">{copy.explain}</span>
                    </button>
                  );
                })}
              </div>
            </fieldset>

            <section className="space-y-2" aria-labelledby="deployment-heading">
              <h3 id="deployment-heading" className="text-sm font-medium">How the instance will run</h3>
              <p className="text-xs text-muted-foreground">
                Agora picks the way the game runs from the list below. Automatic is the default. You can change this later, on the instance's Overview.
              </p>
              <ul className="space-y-1 text-xs text-muted-foreground">
                {DEPLOYMENT_OPTIONS.map((option) => (
                  <li key={option.id}>
                    <span className="font-semibold text-foreground">{option.label}.</span> {option.explain}
                  </li>
                ))}
              </ul>
            </section>

            {creating && (
              <p role="status" className="text-sm">
                Building the base… {progress !== null ? progressText(progress.filesDone, progress.filesTotal) : 'Starting…'}
              </p>
            )}

            {error !== null && (
              <p role="alert" className="text-sm text-destructive">{error}</p>
            )}

            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={creating}>
                Cancel
              </Button>
              <Button type="submit" disabled={!canCreate}>
                {creating ? 'Creating…' : 'Create instance'}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
