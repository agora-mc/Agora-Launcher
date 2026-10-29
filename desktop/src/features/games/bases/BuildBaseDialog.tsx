/**
 * Build a pinned base from an installed copy of the game (MASTER_SPEC §26.4). The user picks the
 * install and the mode, sees what each mode costs, and watches the hashing. A Linked build that
 * cannot link offers a Copied build with its cost shown; it never switches on its own.
 */

import { useEffect, useState } from 'react';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { formatBytes } from '@/components/installed-content/contentTableState';
import { listInstalledGames } from '../discovery/api';
import type { InstallSummary } from '../shared/types';
import { isGamesError } from '../shared/types';
import {
  buildBase,
  listenBuildProgress,
  type BaseMode,
  type BuildProgressEvent,
  type BuildReply,
} from './api';
import {
  copiedNeededText,
  LINK_UNAVAILABLE_TITLE,
  MODE_EXPLANATIONS,
  modeLabel,
  progressPercent,
  progressText,
} from './basesText';

type Installs =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'loaded'; installs: InstallSummary[] };

type Phase =
  | { kind: 'choose' }
  | { kind: 'building'; mode: BaseMode }
  | { kind: 'link_unavailable'; reply: Extract<BuildReply, { status: 'link_unavailable' }> }
  | { kind: 'done'; reply: Exclude<BuildReply, { status: 'link_unavailable' }> };

interface BuildBaseDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  gameId: string;
  /** Called once a base exists (built now, or already there), with the line the list shows. */
  onBuilt: (message: string) => void;
}

function describeFailure(error: unknown): string {
  return isGamesError(error) ? error.message : 'Agora could not build the base. Try again.';
}

export function BuildBaseDialog({ open, onOpenChange, gameId, onBuilt }: BuildBaseDialogProps) {
  const [installs, setInstalls] = useState<Installs>({ status: 'loading' });
  const [selected, setSelected] = useState<string | null>(null);
  const [mode, setMode] = useState<BaseMode>('linked');
  const [phase, setPhase] = useState<Phase>({ kind: 'choose' });
  const [progress, setProgress] = useState<BuildProgressEvent | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Each time the dialog opens, look at the installs again: the user may have just installed or removed one.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setInstalls({ status: 'loading' });
    setError(null);
    setPhase({ kind: 'choose' });
    listInstalledGames()
      .then((list) => {
        if (cancelled) return;
        const game = list.games.find((listing) => listing.gameId === gameId);
        const found = game ? game.installs : [];
        setInstalls({ status: 'loaded', installs: found });
        const usable = found.find((install) => install.runtime.status === 'identified');
        setSelected((current) => (current && found.some((i) => i.installId === current) ? current : usable?.installId ?? null));
      })
      .catch((failure: unknown) => {
        if (!cancelled) setInstalls({ status: 'error', message: describeFailure(failure) });
      });
    return () => {
      cancelled = true;
    };
  }, [open, gameId]);

  const building = phase.kind === 'building';
  const chosen = installs.status === 'loaded' ? installs.installs.find((install) => install.installId === selected) : undefined;

  async function start(chosenMode: BaseMode) {
    if (!chosen) return;
    setError(null);
    setProgress(null);
    setPhase({ kind: 'building', mode: chosenMode });
    let stopListening: (() => void) | null = null;
    try {
      // Listen before the build starts, so no progress event is missed.
      const unlisten = await listenBuildProgress((event) => {
        if (event.installId === chosen.installId) setProgress(event);
      });
      stopListening = () => {
        unlisten();
      };
      const reply = await buildBase(chosen.installId, chosenMode);
      if (reply.status === 'link_unavailable') {
        setPhase({ kind: 'link_unavailable', reply });
        return;
      }
      setPhase({ kind: 'done', reply });
      onBuilt(
        reply.status === 'built'
          ? `Built the ${chosen.store} base for ${chosen.name} (${modeLabel(reply.mode)}).`
          : `A ${chosen.store} base for this version already exists, so nothing was copied.`,
      );
    } catch (failure: unknown) {
      setError(describeFailure(failure));
      setPhase({ kind: 'choose' });
    } finally {
      stopListening?.();
    }
  }

  function requestClose(next: boolean) {
    // A build cannot be stopped once it starts, so the window stays until it finishes.
    if (!next && building) return;
    onOpenChange(next);
  }

  return (
    <Dialog open={open} onOpenChange={requestClose}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Build a pinned base</DialogTitle>
          <DialogDescription>
            Copies one installed version of the game into Agora&apos;s own folder. Instances built from the base run
            from it, so your store can update the game without breaking them.
          </DialogDescription>
        </DialogHeader>

        {phase.kind === 'done' && (
          <BuildDone reply={phase.reply} />
        )}

        {phase.kind === 'link_unavailable' && (
          <div role="alert" className="space-y-3 rounded-lg border border-border p-4 text-sm" data-testid="link-unavailable">
            <p className="font-medium">{LINK_UNAVAILABLE_TITLE}</p>
            <p>{phase.reply.reason}.</p>
            <p>{copiedNeededText(phase.reply.copiedBytesNeeded)} Nothing was built yet.</p>
            <p className="text-muted-foreground">
              You can build a Copied base instead. It takes that much disk space, and the cost is shown here before you agree.
            </p>
          </div>
        )}

        {(phase.kind === 'choose' || phase.kind === 'building') && installs.status === 'loading' && (
          <p role="status" className="text-sm text-muted-foreground">Looking for installed copies…</p>
        )}

        {(phase.kind === 'choose' || phase.kind === 'building') && installs.status === 'error' && (
          <p role="alert" className="text-sm text-destructive">{installs.message}</p>
        )}

        {(phase.kind === 'choose' || phase.kind === 'building') && installs.status === 'loaded' && installs.installs.length === 0 && (
          <div className="space-y-2 rounded-lg border border-dashed border-border p-4 text-sm text-muted-foreground" data-testid="no-installs">
            <p className="font-medium text-foreground">Agora has not found an installed copy of this game.</p>
            <p>Check that the game is installed, then rescan installs in the Installs section above.</p>
          </div>
        )}

        {(phase.kind === 'choose' || phase.kind === 'building') && installs.status === 'loaded' && installs.installs.length > 0 && (
          <div className="space-y-5">
            <fieldset disabled={building} className="space-y-2">
              <legend className="mb-2 text-sm font-medium">Which installed copy?</legend>
              {installs.installs.map((install) => {
                const identified = install.runtime.status === 'identified';
                const pressed = install.installId === selected;
                return (
                  <button
                    key={install.installId}
                    type="button"
                    aria-pressed={pressed}
                    disabled={!identified}
                    onClick={() => setSelected(install.installId)}
                    className={`w-full rounded-lg border p-3 text-left text-sm disabled:opacity-60 ${pressed ? 'border-primary bg-accent' : 'border-border hover:bg-accent/50'}`}
                  >
                    <span className="font-medium">{install.name}</span>
                    <span className="ml-2 text-muted-foreground">
                      {install.store}{install.storeVersion ? `, version ${install.storeVersion}` : ''}
                    </span>
                    <span className="block break-all font-mono text-xs text-muted-foreground">{install.location}</span>
                    {!identified && install.runtime.status === 'unidentified' && (
                      <span className="mt-1 block text-destructive">
                        Agora cannot tell which exact version this is, so it cannot build from it. {install.runtime.reasons.join(' ')}
                      </span>
                    )}
                  </button>
                );
              })}
            </fieldset>

            <fieldset disabled={building} className="space-y-2">
              <legend className="mb-2 text-sm font-medium">How should the base be built?</legend>
              {(['linked', 'copied'] as const).map((choice) => (
                <button
                  key={choice}
                  type="button"
                  aria-pressed={mode === choice}
                  onClick={() => setMode(choice)}
                  className={`w-full rounded-lg border p-3 text-left text-sm ${mode === choice ? 'border-primary bg-accent' : 'border-border hover:bg-accent/50'}`}
                >
                  <span className="font-medium">{MODE_EXPLANATIONS[choice].title}</span>
                  <span className="mt-1 block text-muted-foreground">{MODE_EXPLANATIONS[choice].body}</span>
                </button>
              ))}
            </fieldset>

            {building && (
              <div className="space-y-2" data-testid="build-progress">
                <div
                  role="progressbar"
                  aria-label="Building the base"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={progress ? progressPercent(progress) : undefined}
                  className="h-2 w-full overflow-hidden rounded-full bg-muted"
                >
                  <div
                    className="h-full bg-primary transition-[width]"
                    style={{ width: `${progress ? progressPercent(progress) : 0}%` }}
                  />
                </div>
                <p role="status" className="text-sm text-muted-foreground">
                  {progress ? progressText(progress) : 'Starting. Reading the game files…'}
                </p>
                <p className="text-xs text-muted-foreground">
                  Hashing a large game takes a few minutes. This cannot be stopped once it starts.
                </p>
              </div>
            )}

            {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
          </div>
        )}

        <DialogFooter className="gap-2">
          {phase.kind === 'done' && <Button type="button" onClick={() => onOpenChange(false)}>Done</Button>}

          {phase.kind === 'link_unavailable' && (
            <>
              <Button type="button" variant="outline" onClick={() => setPhase({ kind: 'choose' })}>Back</Button>
              <Button type="button" onClick={() => start('copied')}>
                Build a Copied base ({formatBytes(phase.reply.copiedBytesNeeded)})
              </Button>
            </>
          )}

          {(phase.kind === 'choose' || phase.kind === 'building') && (
            <>
              <Button type="button" variant="outline" disabled={building} onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button
                type="button"
                disabled={!chosen || building}
                onClick={() => start(mode)}
              >
                {`Build a ${modeLabel(mode)} base`}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function BuildDone({ reply }: { reply: Exclude<BuildReply, { status: 'link_unavailable' }> }) {
  if (reply.status === 'existing') {
    return (
      <p role="status" className="rounded-lg border border-border p-4 text-sm">
        A base for this exact version already exists, so nothing was copied. It is in the list below.
      </p>
    );
  }
  return (
    <div role="status" className="space-y-1 rounded-lg border border-border p-4 text-sm">
      <p className="font-medium">Built the {modeLabel(reply.mode)} base.</p>
      <p>
        {reply.fileCount} files. {formatBytes(reply.linkedBytes)} is shared with the store install, which takes no extra
        space. {formatBytes(reply.copiedBytes)} is the base&apos;s own.
      </p>
    </div>
  );
}
