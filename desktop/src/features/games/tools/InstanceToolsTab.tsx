/**
 * An instance's Tools page (MASTER_SPEC §26.9): the tools the game declares, the output each has
 * here, and the swap banner when a swap was interrupted. Each tool's own actions live in ToolCard.
 */

import { useCallback, useEffect, useState } from 'react';
import { gameToolSwapStatus, listGameTools, restoreGameToolSwap, type SwapStatus, type ToolsView } from './api';
import { ToolCard } from './ToolCard';
import { conflictLine, messageOf } from './toolCopy';

export function InstanceToolsTab({ instanceId }: { gameId: string; instanceId: string }) {
  const [view, setView] = useState<ToolsView | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [swap, setSwap] = useState<SwapStatus | null>(null);
  const [swapError, setSwapError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [restoreMessage, setRestoreMessage] = useState<string | null>(null);

  const load = useCallback(async (isCurrent: () => boolean) => {
    setLoadError(null);
    setSwapError(null);
    const [listed, status] = await Promise.allSettled([listGameTools(instanceId), gameToolSwapStatus(instanceId)]);
    if (!isCurrent()) return;
    if (listed.status === 'fulfilled') setView(listed.value);
    else { setView(null); setLoadError(messageOf(listed.reason)); }
    if (status.status === 'fulfilled') setSwap(status.value);
    else { setSwap(null); setSwapError(messageOf(status.reason)); }
  }, [instanceId]);

  useEffect(() => {
    let current = true;
    void load(() => current);
    return () => { current = false; };
  }, [load]);

  const reload = () => { void load(() => true); };

  const restore = async () => {
    setBusy(true);
    setRestoreMessage(null);
    try {
      const reply = await restoreGameToolSwap(instanceId);
      setRestoreMessage(reply.restoredTools.length > 0
        ? `The game’s Data folder is back where it belongs${reply.restoredTools.length === 1 ? ` (the swap for ${reply.restoredTools[0]})` : ''}.`
        : 'Nothing needed putting back.');
      reload();
    } catch (error) {
      setRestoreMessage(messageOf(error));
    } finally {
      setBusy(false);
    }
  };

  const interrupted = swap?.interrupted ?? null;

  return (
    <section className="space-y-4" aria-labelledby="tools-heading" data-testid="instance-tools">
      <header className="space-y-1">
        <h2 id="tools-heading" className="text-lg font-semibold">Tools</h2>
        <p className="text-sm text-muted-foreground">
          Tools write files the game reads, such as animation or body files. Each run’s output is kept as a
          generation, so you can roll back, compare or remove it without touching your mods.
        </p>
      </header>

      {swapError && (
        <p role="alert" className="rounded-lg border border-destructive bg-destructive/10 p-3 text-sm">
          Agora could not check for an interrupted swap, so it cannot say whether the game’s Data folder is in
          place: {swapError}
        </p>
      )}

      {interrupted && (
        <section role="alert" aria-label="Interrupted tool swap" className="space-y-2 rounded-xl border border-destructive bg-destructive/10 p-4 text-sm" data-testid="tool-swap-recovery">
          <h3 className="font-semibold">A tool swap was interrupted</h3>
          <p>
            The game’s real Data folder may be renamed aside while a tool runs on a copy. Put it back before you launch
            the game or run another tool.
          </p>
          <p>
            {interrupted.tool
              ? `The swap was for ${interrupted.tool}.`
              : 'Agora cannot read which tool the swap was for.'}
            {interrupted.unreadable ? ` The saved record cannot be read: ${interrupted.unreadable}` : ''}
          </p>
          {interrupted.running ? (
            <p>A tool is still running from the game folder. Let it finish or close it, then try again.</p>
          ) : (
            <button
              type="button"
              disabled={busy}
              onClick={() => void restore()}
              className="rounded-lg bg-primary px-3 py-1.5 font-medium text-primary-foreground shadow-sm disabled:opacity-50"
            >
              Put the Data folder back
            </button>
          )}
        </section>
      )}
      {restoreMessage && <p role="status" className="text-sm text-muted-foreground">{restoreMessage}</p>}

      {!view && !loadError && <p className="text-sm text-muted-foreground">Looking at this instance’s tools…</p>}

      {loadError && (
        <div role="alert" className="space-y-2 rounded-xl border border-destructive bg-destructive/10 p-4 text-sm">
          <p>The tools could not be listed: {loadError}</p>
          <button
            type="button"
            onClick={reload}
            className="rounded-lg border border-border px-3 py-1.5 font-medium hover:bg-accent"
          >
            Try again
          </button>
        </div>
      )}

      {view && view.tools.length === 0 && (
        <p className="rounded-xl border border-dashed border-border p-6 text-sm text-muted-foreground" data-testid="tools-empty">
          This game declares no tools, so there is nothing here to build. Tools appear when a game package provides them.
        </p>
      )}

      {view && view.tools.map((row) => (
        <div key={row.toolId} className="space-y-2">
          {row.conflicts && (
            <p role="alert" className="rounded-lg border border-destructive bg-destructive/10 p-3 text-sm" data-testid="tool-conflict">
              {conflictLine(row, view.tools)}
            </p>
          )}
          <ToolCard
            instanceId={instanceId}
            row={row}
            busy={busy}
            setBusy={setBusy}
            onView={setView}
            onChanged={reload}
          />
        </div>
      ))}
    </section>
  );
}
