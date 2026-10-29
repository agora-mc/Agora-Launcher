import { useCallback, useEffect, useState } from 'react';
import { Boxes, Play, Plus, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useConfirm } from '@/components/ui/confirm';
import { CreateInstanceDialog } from './CreateInstanceDialog';
import {
  deleteGameInstance,
  gamesErrorMessage,
  listGameInstances,
  type InstanceCard,
} from './api';
import { deploymentLabel, formatWhen } from './instanceText';

interface GameInstancesPageProps {
  gameId: string;
  /** Open an instance's page. The shell owns the navigation; the slice only calls this. */
  onOpenInstance: (instanceId: string) => void;
}

type LoadState =
  | { status: 'loading' }
  | { status: 'failed'; message: string }
  | { status: 'loaded'; instances: InstanceCard[] };

/** The Instances tab of a non-Minecraft game: its instances as cards, with create and delete. */
export function GameInstancesPage({ gameId, onOpenInstance }: GameInstancesPageProps) {
  const { confirm } = useConfirm();
  const [state, setState] = useState<LoadState>({ status: 'loading' });
  const [createOpen, setCreateOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const load = useCallback(() => {
    setState({ status: 'loading' });
    listGameInstances(gameId)
      .then((list) => setState({ status: 'loaded', instances: list.instances }))
      .catch((error: unknown) => setState({ status: 'failed', message: gamesErrorMessage(error) }));
  }, [gameId]);

  useEffect(() => {
    load();
  }, [load]);

  const remove = async (card: InstanceCard) => {
    const approved = await confirm({
      title: `Delete ${card.name}?`,
      body: 'This removes the instance and its folder. The game install and the pinned base are kept.',
      confirmLabel: 'Delete instance',
      tone: 'danger',
    });
    if (!approved) return;
    setBusyId(card.instanceId);
    setActionError(null);
    setNotice(null);
    try {
      const deleted = await deleteGameInstance(card.instanceId);
      setNotice(
        deleted.orphanedBase === null
          ? `${card.name} was deleted.`
          : `${card.name} was deleted. The base ${deleted.orphanedBase} is no longer used by any instance. Remove it under Setup, pinned bases, if you want the disk space back.`,
      );
      load();
    } catch (error) {
      setActionError(`${card.name} was not deleted. ${gamesErrorMessage(error)}`);
    } finally {
      setBusyId(null);
    }
  };

  return (
    <section className="space-y-6" data-testid="game-instances-tab">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-2xl font-bold tracking-tight">Instances</h2>
          <p className="text-sm text-muted-foreground">
            Each instance is its own setup of this game: its mods, its load order and its saves.
          </p>
        </div>
        <Button type="button" onClick={() => setCreateOpen(true)} disabled={state.status !== 'loaded'}>
          <Plus aria-hidden="true" />
          New instance
        </Button>
      </header>

      {notice !== null && (
        <p role="status" className="rounded-lg border border-border bg-card p-3 text-sm">
          {notice}
        </p>
      )}
      {actionError !== null && (
        <p role="alert" className="rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
          {actionError}
        </p>
      )}

      {state.status === 'loading' && (
        <p className="text-sm text-muted-foreground" role="status">Loading instances…</p>
      )}

      {state.status === 'failed' && (
        <div role="alert" className="space-y-3 rounded-xl border border-destructive/50 bg-destructive/10 p-6">
          <h3 className="font-semibold">The instances could not be loaded</h3>
          <p className="text-sm">{state.message}</p>
          <Button type="button" variant="outline" onClick={load}>Try again</Button>
        </div>
      )}

      {state.status === 'loaded' && state.instances.length === 0 && (
        <div className="space-y-3 rounded-xl border border-dashed border-border bg-card/60 p-8 text-center" data-testid="instances-empty">
          <Boxes className="mx-auto size-10 text-muted-foreground" aria-hidden="true" />
          <h3 className="text-lg font-semibold">No instances yet</h3>
          <p className="mx-auto max-w-md text-sm text-muted-foreground">
            An instance starts from an install of this game that Agora found on this machine. Create one, then add mods to it.
          </p>
          <Button type="button" onClick={() => setCreateOpen(true)}>
            <Plus aria-hidden="true" />
            Create the first instance
          </Button>
        </div>
      )}

      {state.status === 'loaded' && state.instances.length > 0 && (
        <ul className="grid gap-4 md:grid-cols-2" aria-label="Instances">
          {state.instances.map((card) => (
            <li key={card.instanceId} className="flex flex-col gap-3 rounded-xl border border-border bg-card p-5" data-testid="instance-card">
              <div className="flex flex-wrap items-start justify-between gap-2">
                <h3 className="text-lg font-semibold">{card.name}</h3>
                <div className="flex flex-wrap gap-1.5">
                  {card.running && (
                    <span className="rounded-full bg-primary px-2.5 py-0.5 text-xs font-semibold text-primary-foreground">Running</span>
                  )}
                  <span className="rounded-full border border-border px-2.5 py-0.5 text-xs font-semibold">
                    {card.pinned ? 'Pinned base' : 'Runs from install'}
                  </span>
                </div>
              </div>
              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
                <dt className="text-muted-foreground">Runtime</dt>
                <dd>{[card.store, card.version].filter(Boolean).join(' ') || 'Not recorded'}</dd>
                <dt className="text-muted-foreground">Deployment</dt>
                <dd>{deploymentLabel(card.deployment)}</dd>
                <dt className="text-muted-foreground">Last played</dt>
                <dd>{formatWhen(card.lastLaunchedAt)}</dd>
              </dl>
              <div className="mt-auto flex flex-wrap gap-2">
                <Button type="button" onClick={() => onOpenInstance(card.instanceId)}>
                  <Play aria-hidden="true" />
                  Open
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => void remove(card)}
                  disabled={busyId !== null || card.running}
                  aria-label={`Delete ${card.name}`}
                >
                  <Trash2 aria-hidden="true" />
                  Delete
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}

      <CreateInstanceDialog
        open={createOpen}
        gameId={gameId}
        onOpenChange={setCreateOpen}
        onCreated={(instanceId) => {
          setCreateOpen(false);
          onOpenInstance(instanceId);
        }}
      />
    </section>
  );
}
