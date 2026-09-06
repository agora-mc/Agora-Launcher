import { useCallback, useEffect, useState } from 'react';
import { Bug } from 'lucide-react';
import {
  applyBisectTrial,
  cancelBisect,
  formatError,
  getBisectSession,
  recordBisectOutcome,
  startBisect,
  stepBackBisect,
  type BisectView,
} from '@/lib/tauri';
import { useConfirm } from '@/components/ui/confirm';

/**
 * Drive a guided mod bisect.
 *
 * "Disable half your mods and launch" is the most-repeated piece of Minecraft
 * troubleshooting advice and the most commonly abandoned, because keeping track
 * across a dozen launches is genuinely hard. The session is persisted, so the
 * user can close the launcher between trials and still step back to take the
 * half they skipped.
 */
export function ModBisectPanel({
  instanceId,
  primeSuspects = [],
  locked = false,
  onLaunch,
}: {
  instanceId: string;
  /** Mods the crash log implicated — tested first. */
  primeSuspects?: string[];
  locked?: boolean;
  /** Start the game after a trial is applied. Without it the button would only
   *  toggle files and the user would have to launch by hand. */
  onLaunch?: () => Promise<unknown>;
}) {
  const { confirm } = useConfirm();
  const [view, setView] = useState<BisectView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setView(await getBisectSession(instanceId));
    } catch (e) {
      setError(formatError(e));
    }
  }, [instanceId]);

  useEffect(() => { void refresh(); }, [refresh]);

  /**
   * `changesContent` covers every action that renames JARs on disk.
   *
   * Crash Doctor is a global overlay, so the instance editor underneath it has
   * no idea a trial just toggled half the mod list and keeps rendering the
   * pre-trial arrangement — which reads as "restore my mods did nothing" even
   * when the files on disk are correct. A window event is the cheapest way to
   * reach across that boundary; the editor listens and refetches.
   */
  const run = async (
    action: () => Promise<BisectView | void>,
    changesContent = false,
  ) => {
    setBusy(true);
    setError(null);
    try {
      const next = await action();
      if (next) setView(next);
      else await refresh();
      if (changesContent) {
        window.dispatchEvent(new CustomEvent('agora-instance-content-changed', {
          detail: { instanceId },
        }));
      }
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const session = view?.session ?? null;
  const trial = view?.trial ?? null;
  const status = trial?.status;

  return (
    <section className="rounded-xl border border-border bg-card p-4 space-y-3">
      <div className="flex items-center gap-2">
        <Bug className="h-4 w-4 text-muted-foreground" aria-hidden="true" />
        <h3 className="font-semibold text-sm">Find the mod that breaks it</h3>
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}

      {!session && (
        <>
          <p className="text-sm text-muted-foreground">
            Agora will turn off half your mods at a time and ask you to launch. Each answer halves
            what is left, so it usually takes a handful of tries rather than dozens.
            {primeSuspects.length > 0 && ' The crash report already named some suspects — those go first.'}
          </p>
          <button
            type="button"
            onClick={() => void run(() => startBisect(instanceId, primeSuspects))}
            disabled={busy || locked}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy ? 'Starting…' : 'Start'}
          </button>
        </>
      )}

      {session && status?.type === 'awaiting_trial' && trial && (
        <>
          <p className="text-sm">
            Trial {trial.completed_trials + 1} — {session.suspects.length} mods still suspected,
            about {trial.remaining_trials} more {trial.remaining_trials === 1 ? 'try' : 'tries'} to go.
          </p>
          <p className="text-xs text-muted-foreground">
            {trial.disable.length} mod{trial.disable.length === 1 ? '' : 's'} will be turned off.
            Anything that depends on them is turned off too — otherwise the game would crash on the
            missing dependency instead of the bug.
          </p>
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => void run(async () => {
                const next = await applyBisectTrial(instanceId);
                // Applying without launching leaves the user staring at a
                // button that appears to have done nothing, since the change
                // is a set of file renames they cannot see.
                await onLaunch?.();
                return next;
              }, true)}
              disabled={busy || locked}
              className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
            >
              {busy ? 'Applying…' : onLaunch ? 'Apply and launch' : 'Apply this trial'}
            </button>
            <span className="self-center text-xs text-muted-foreground">then tell us what happened:</span>
            <button
              type="button"
              onClick={() => void run(() => recordBisectOutcome(instanceId, true))}
              disabled={busy}
              className="rounded-lg border border-input bg-background px-3 py-2 text-sm hover:bg-accent disabled:opacity-50"
            >
              Still broken
            </button>
            <button
              type="button"
              onClick={() => void run(() => recordBisectOutcome(instanceId, false))}
              disabled={busy}
              className="rounded-lg border border-input bg-background px-3 py-2 text-sm hover:bg-accent disabled:opacity-50"
            >
              Worked fine
            </button>
          </div>
        </>
      )}

      {status?.type === 'culprit' && (
        <p className="text-sm">
          <span className="font-medium">{status.filename}</span> is the one. Removing or updating it
          should fix the problem.
        </p>
      )}

      {status?.type === 'culprit_group' && (
        <div className="text-sm">
          <p>Narrowed to these, which have to be turned off together:</p>
          <ul className="mt-1 list-disc pl-5 text-muted-foreground">
            {status.filenames.map((filename) => <li key={filename}>{filename}</li>)}
          </ul>
        </div>
      )}

      {status?.type === 'inconclusive' && (
        <p className="text-sm text-muted-foreground">
          Every mod was cleared, so this is not one mod on its own — it may be a combination, a
          config file, or something outside the mod list entirely.
        </p>
      )}

      {session && (
        <div className="flex gap-3 border-t border-border pt-3 text-xs">
          <button
            type="button"
            onClick={() => void run(() => stepBackBisect(instanceId), true)}
            disabled={busy || session.history.length === 0}
            className="text-foreground hover:underline disabled:opacity-40"
            title="Undo the last answer and try the other half instead"
          >
            Step back
          </button>
          <button
            type="button"
            onClick={() => void (async () => {
              if (!await confirm({
                title: 'Stop the bisect and turn every mod back on as it was?',
                confirmLabel: 'Stop and restore',
              })) return;
              await run(async () => { await cancelBisect(instanceId); }, true);
            })()}
            disabled={busy}
            className="ml-auto text-destructive hover:underline disabled:opacity-40"
          >
            Stop and restore my mods
          </button>
        </div>
      )}
    </section>
  );
}
