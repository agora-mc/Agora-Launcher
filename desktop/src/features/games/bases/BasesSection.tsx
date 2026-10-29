/**
 * The Setup tab's pinned bases (MASTER_SPEC §26.4): the bases built for this game, each with its
 * checks and its removal, and a way to build a new one.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { useConfirm } from '@/components/ui/confirm';
import { isGamesError } from '../shared/types';
import { listBases, removeBase, type BaseSummary } from './api';
import { BaseCard } from './BaseCard';
import { BuildBaseDialog } from './BuildBaseDialog';

type Load =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'loaded'; bases: BaseSummary[] };

/** A backend error shows its own message; anything else is a reply Agora could not read. */
export function errorMessage(error: unknown): string {
  return isGamesError(error) ? error.message : 'Agora could not read the list of bases. Try again.';
}

interface BasesSectionProps {
  gameId: string;
}

export function BasesSection({ gameId }: BasesSectionProps) {
  const [load, setLoad] = useState<Load>({ status: 'loading' });
  const [notice, setNotice] = useState<string | null>(null);
  const [buildOpen, setBuildOpen] = useState(false);
  const latest = useRef(0);
  const { confirm } = useConfirm();

  // Only the newest request may answer: a slow list must not overwrite a newer one.
  const refresh = useCallback(() => {
    const request = ++latest.current;
    listBases(gameId)
      .then((list) => {
        if (request === latest.current) setLoad({ status: 'loaded', bases: list.bases });
      })
      .catch((error: unknown) => {
        if (request === latest.current) setLoad({ status: 'error', message: errorMessage(error) });
      });
  }, [gameId]);

  useEffect(() => {
    setNotice(null);
    setLoad({ status: 'loading' });
    refresh();
  }, [refresh]);

  async function remove(base: BaseSummary) {
    const ok = await confirm({
      title: `Remove the ${base.store} ${base.version} base?`,
      body: 'Its folder and record are deleted. The game in your store is not touched, and you can build the base again later.',
      confirmLabel: 'Remove base',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await removeBase(base.baseId);
      setNotice(`Removed the ${base.store} ${base.version} base.`);
      refresh();
    } catch (error: unknown) {
      // The backend names the instances that still use the base; show its message as it is.
      setNotice(isGamesError(error) ? error.message : 'Agora could not remove the base. Try again.');
    }
  }

  return (
    <Card data-testid="setup-bases">
      <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-4">
        <div className="space-y-1.5">
          <CardTitle>Pinned bases</CardTitle>
          <CardDescription>
            A base is a verified copy of one version of the game. Instances built from it keep working when your store
            updates the game. Bases no instance uses can be removed.
          </CardDescription>
        </div>
        <Button
          type="button"
          onClick={() => setBuildOpen(true)}
          disabled={load.status === 'loading'}
        >
          Build a base
        </Button>
      </CardHeader>
      <CardContent className="space-y-4">
        {notice && (
          <p role="status" className="rounded-lg border border-border bg-muted/40 p-3 text-sm">
            {notice}
          </p>
        )}

        {load.status === 'loading' && (
          <p role="status" className="text-sm text-muted-foreground">Loading bases…</p>
        )}

        {load.status === 'error' && (
          <div role="alert" className="space-y-3 rounded-lg border border-destructive/50 p-4 text-sm">
            <p>{load.message}</p>
            <Button type="button" variant="outline" size="sm" onClick={() => {
              setLoad({ status: 'loading' });
              refresh();
            }}
            >
              Try again
            </Button>
          </div>
        )}

        {load.status === 'loaded' && load.bases.length === 0 && (
          <div className="space-y-2 rounded-lg border border-dashed border-border p-4 text-sm text-muted-foreground" data-testid="bases-empty">
            <p className="font-medium text-foreground">No pinned bases yet.</p>
            <p>
              Build one from an installed copy of the game. Instances pinned to it run from their own verified copy,
              so a store update cannot break them.
            </p>
          </div>
        )}

        {load.status === 'loaded' && load.bases.length > 0 && (
          <ul className="space-y-3" aria-label="Pinned bases">
            {load.bases.map((base) => (
              <li key={base.baseId}>
                <BaseCard base={base} onRemove={() => remove(base)} />
              </li>
            ))}
          </ul>
        )}
      </CardContent>

      <BuildBaseDialog
        open={buildOpen}
        onOpenChange={setBuildOpen}
        gameId={gameId}
        onBuilt={(message) => {
          setNotice(message);
          refresh();
        }}
      />
    </Card>
  );
}
