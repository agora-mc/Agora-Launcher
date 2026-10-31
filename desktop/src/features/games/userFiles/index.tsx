/**
 * User-files slot owned by the user-files slice: the Setup tab's recovery section.
 *
 * When Agora swapped the game's own files for an instance's copies, it kept a journal. This
 * shows any session still in progress or left over, and puts the game's files back from the
 * backup on request. Restore is refused while the game runs, and asks first.
 */

import { useEffect, useState } from 'react';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { useConfirm } from '@/components/ui/confirm';
import { formatError } from '@/lib/tauri';
import { restoreUserFiles, userFilesStatus, type RestoreResult, type UserFileSession } from './api';

type ListState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'loaded'; sessions: UserFileSession[] };

type RestoreState =
  | { status: 'busy' }
  | { status: 'error'; message: string }
  | { status: 'done'; result: RestoreResult };

export function SetupRecoverySection({ gameId }: { gameId: string }) {
  const { confirm } = useConfirm();
  const [list, setList] = useState<ListState>({ status: 'loading' });
  const [restores, setRestores] = useState<Record<string, RestoreState>>({});

  const load = () => {
    setList({ status: 'loading' });
    userFilesStatus(gameId)
      .then((reply) => setList({ status: 'loaded', sessions: reply.sessions }))
      .catch((error: unknown) => setList({ status: 'error', message: formatError(error) }));
  };

  useEffect(() => {
    load();
    // Loaded again only when the game changes; "Try again" calls load().
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gameId]);

  const restore = async (session: UserFileSession) => {
    const key = `${session.gameId}/${session.store}`;
    const ok = await confirm({
      title: 'Put the game’s own files back?',
      body: `Agora puts back the game’s own files it swapped out for this ${session.store} session. Anything written to those files since the swap is first saved into the instance’s copy, so nothing is lost.`,
      confirmLabel: 'Restore files',
      tone: 'danger',
    });
    if (!ok) return;
    setRestores((current) => ({ ...current, [key]: { status: 'busy' } }));
    try {
      const result = await restoreUserFiles(session.gameId, session.store);
      setRestores((current) => ({ ...current, [key]: { status: 'done', result } }));
      load();
    } catch (error) {
      setRestores((current) => ({ ...current, [key]: { status: 'error', message: formatError(error) } }));
    }
  };

  return (
    <section className="space-y-4 rounded-xl border border-border bg-card p-5" aria-labelledby="recovery-heading" data-testid="recovery-section">
      <header className="space-y-1">
        <h3 id="recovery-heading" className="text-base font-semibold">
          Recovery
        </h3>
        <p className="text-sm text-muted-foreground">
          When Agora swaps a game’s own files for an instance’s copies, it keeps a backup. A session here is one of those
          swaps that has not been put back yet.
        </p>
      </header>

      {list.status === 'loading' && (
        <p role="status" className="text-sm text-muted-foreground">
          Checking for swapped files.
        </p>
      )}

      {list.status === 'error' && (
        <div role="alert" className="space-y-2 text-sm">
          <p className="text-destructive">Agora could not check for swapped files: {list.message}</p>
          <Button variant="outline" size="sm" onClick={load}>
            Try again
          </Button>
        </div>
      )}

      {list.status === 'loaded' && list.sessions.length === 0 && (
        <p className="text-sm text-muted-foreground" data-testid="recovery-empty">
          Nothing needs recovering. No game’s files are swapped right now.
        </p>
      )}

      {list.status === 'loaded' &&
        list.sessions.map((session) => {
          const key = `${session.gameId}/${session.store}`;
          const restoreState = restores[key];
          const swapped = session.files.filter((file) => file.swapped).length;
          return (
            <div key={key} className="space-y-3 rounded-lg border border-border p-4 text-sm" data-testid="recovery-session">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div>
                  <p className="font-medium">
                    {session.store} copy of {session.gameId}
                  </p>
                  <p className="text-muted-foreground">
                    Instance {session.instanceId} · {session.files.length} file(s), {swapped} swapped
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  {session.running ? (
                    <Badge variant="secondary">Game is running</Badge>
                  ) : (
                    <Button size="sm" onClick={() => restore(session)} disabled={restoreState?.status === 'busy'}>
                      {restoreState?.status === 'busy' ? 'Restoring…' : 'Put the files back'}
                    </Button>
                  )}
                </div>
              </div>
              {session.running && (
                <p className="text-muted-foreground">
                  Close the game before putting its files back. Agora will not restore files a running game is using.
                </p>
              )}
              <ul className="space-y-0.5 font-mono text-xs text-muted-foreground">
                {session.files.map((file) => (
                  <li key={file.realPath}>
                    {file.realPath} {file.swapped ? '(swapped)' : '(not swapped)'}
                  </li>
                ))}
              </ul>
              {restoreState?.status === 'error' && (
                <p role="alert" className="text-destructive">
                  The files were not put back: {restoreState.message}
                </p>
              )}
              {restoreState?.status === 'done' && (
                <div className="space-y-1" data-testid="recovery-restored">
                  <p>
                    Put back {restoreState.result.files.length} file(s) from the instance {restoreState.result.instanceId}.
                  </p>
                  {restoreState.result.files
                    .filter((file) => file.changed)
                    .map((file) => (
                      <p key={file.realPath} className="text-amber-700 dark:text-amber-300">
                        Changed since the swap: {file.realPath}
                      </p>
                    ))}
                </div>
              )}
            </div>
          );
        })}
    </section>
  );
}
