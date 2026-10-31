import { useCallback, useEffect, useState } from 'react';
import { Play, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  checkGameInstance,
  getGameInstance,
  gamesErrorCode,
  gamesErrorMessage,
  launchGameInstance,
  onInstanceExited,
  setGameInstanceDeployment,
  type CheckFinding,
  type DeploymentChoice,
  type DeploymentMode,
  type InstanceCheck,
  type InstanceDetail,
  type LaunchReply,
} from './api';
import {
  checkHeadline,
  DEPLOYMENT_OPTIONS,
  deploymentLabel,
  elapsedText,
  fallbackTradeoffs,
  formatWhen,
  frameworksMatch,
  orderFindings,
} from './instanceText';

interface InstanceOverviewTabProps {
  gameId: string;
  instanceId: string;
}

type PageState =
  | { status: 'loading' }
  | { status: 'failed'; message: string }
  | { status: 'loaded'; detail: InstanceDetail; check: InstanceCheck | null; checkError: string | null };

/** How often the page asks whether the game is still running, while it is running or starting. */
const RUNNING_POLL_MS = 2000;

/** An instance's Overview: its runtime, its launch checks, how it runs, and Launch. */
export function InstanceOverviewTab({ instanceId }: InstanceOverviewTabProps) {
  const [page, setPage] = useState<PageState>({ status: 'loading' });
  const [busy, setBusy] = useState<'deployment' | 'launch' | null>(null);
  const [launchStartedAt, setLaunchStartedAt] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [reply, setReply] = useState<LaunchReply | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [launchAnyway, setLaunchAnyway] = useState(false);

  const load = useCallback(async () => {
    const detail = await getGameInstance(instanceId);
    let check: InstanceCheck | null = null;
    let checkError: string | null = null;
    try {
      check = await checkGameInstance(instanceId);
    } catch (failure) {
      checkError = gamesErrorMessage(failure);
    }
    setPage({ status: 'loaded', detail, check, checkError });
  }, [instanceId]);

  // Start from nothing on each instance, so a page never shows the last instance's answers. The
  // "Launch anyway" choice goes too: a tick made for one instance must not carry over to another.
  useEffect(() => {
    setPage({ status: 'loading' });
    setReply(null);
    setActionError(null);
    setLaunchAnyway(false);
    load().catch((failure: unknown) => setPage({ status: 'failed', message: gamesErrorMessage(failure) }));
  }, [load]);

  const refreshDetail = useCallback(() => {
    getGameInstance(instanceId)
      .then((detail) => setPage((current) => (current.status === 'loaded' ? { ...current, detail } : current)))
      .catch((failure: unknown) => setActionError(gamesErrorMessage(failure)));
  }, [instanceId]);

  const running = page.status === 'loaded' && page.detail.running;

  // The game's exit is reported by an event; the poll covers a page that opened while it ran.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    onInstanceExited((exited) => {
      if (exited.instanceId !== instanceId) return;
      if (exited.restoreError !== null) {
        setActionError(`The game closed, but Agora could not put your files back: ${exited.restoreError}`);
      }
      refreshDetail();
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [instanceId, refreshDetail]);

  useEffect(() => {
    if (!running && busy !== 'launch') return;
    const timer = window.setInterval(refreshDetail, RUNNING_POLL_MS);
    return () => window.clearInterval(timer);
  }, [running, busy, refreshDetail]);

  useEffect(() => {
    if (busy !== 'launch') return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [busy]);

  const changeDeployment = async (choice: DeploymentChoice) => {
    setBusy('deployment');
    setActionError(null);
    try {
      await setGameInstanceDeployment(instanceId, choice);
    } catch (failure) {
      setActionError(`The way this instance runs was not changed. ${gamesErrorMessage(failure)}`);
      return;
    } finally {
      setBusy(null);
    }
    // The change is saved at this point; a failed refresh must not read as a failed change.
    refreshDetail();
  };

  const launch = async (deployment?: DeploymentMode) => {
    setBusy('launch');
    setLaunchStartedAt(Date.now());
    setReply(null);
    setActionError(null);
    try {
      const answer = await launchGameInstance(instanceId, { launchAnyway, deployment });
      setReply(answer);
      if (answer.status === 'launched') refreshDetail();
    } catch (failure) {
      setActionError(
        gamesErrorCode(failure) === 'instance_running'
          ? 'The game is already running from this instance.'
          : `The game did not start. ${gamesErrorMessage(failure)}`,
      );
      refreshDetail();
    } finally {
      setBusy(null);
      setLaunchStartedAt(null);
    }
  };

  // The question the virtual file system raises: run once from the next rung, always from it, or
  // stop. Nothing runs until the user picks one.
  const answerFallback = async (next: DeploymentMode, always: boolean) => {
    setReply(null);
    if (always) {
      setBusy('deployment');
      try {
        await setGameInstanceDeployment(instanceId, next);
      } catch (failure) {
        setActionError(`The way this instance runs was not changed. ${gamesErrorMessage(failure)}`);
        return;
      } finally {
        setBusy(null);
      }
      await launch();
      return;
    }
    await launch(next);
  };

  if (page.status === 'loading') {
    return <p role="status" className="text-sm text-muted-foreground">Loading this instance…</p>;
  }

  if (page.status === 'failed') {
    return (
      <div role="alert" className="space-y-3 rounded-xl border border-destructive/50 bg-destructive/10 p-6">
        <h2 className="font-semibold">This instance could not be loaded</h2>
        <p className="text-sm">{page.message}</p>
        <Button type="button" variant="outline" onClick={() => void load().catch(() => undefined)}>Try again</Button>
      </div>
    );
  }

  const { detail, check, checkError } = page;
  const matches = frameworksMatch(check);
  const choice: DeploymentChoice = detail.deployment ?? 'auto';
  const choiceCopy = DEPLOYMENT_OPTIONS.find((option) => option.id === choice);
  const elapsed = launchStartedAt === null ? 0 : (now - launchStartedAt) / 1000;

  return (
    <section className="space-y-6" data-testid="instance-overview">
      <header className="space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="text-2xl font-bold tracking-tight">{detail.name}</h2>
          {detail.running && (
            <span className="rounded-full bg-primary px-2.5 py-0.5 text-xs font-semibold text-primary-foreground" role="status">
              Running now
            </span>
          )}
        </div>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
          <dt className="text-muted-foreground">Game</dt>
          <dd>{detail.gameName}</dd>
          <dt className="text-muted-foreground">Runtime</dt>
          <dd>{[detail.store, detail.version, detail.build].filter(Boolean).join(' ') || 'Not recorded'}</dd>
          <dt className="text-muted-foreground">Base</dt>
          <dd>
            {detail.pinned
              ? `Pinned${detail.baseId ? ` (${detail.baseId})` : ''}`
              : `Not pinned: runs from the install folder${detail.unpinnedReason ? `. ${detail.unpinnedReason}` : ''}`}
          </dd>
          <dt className="text-muted-foreground">Frameworks</dt>
          <dd data-testid="frameworks-status">
            {matches === null && 'Not checked'}
            {matches === true && 'Match this game version'}
            {matches === false && 'Do not match this game version'}
          </dd>
        </dl>
      </header>

      {actionError !== null && (
        <p role="alert" className="rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
          {actionError}
        </p>
      )}

      <section aria-labelledby="checks-heading" className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h3 id="checks-heading" className="text-lg font-semibold">Launch checks</h3>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void load().catch((failure: unknown) => setActionError(gamesErrorMessage(failure)))}
          >
            <RefreshCw aria-hidden="true" />
            Check again
          </Button>
        </div>
        {checkError !== null && (
          <p role="alert" className="text-sm text-destructive">The checks could not run. {checkError}</p>
        )}
        {checkError === null && <p className="text-sm text-muted-foreground">{checkHeadline(check)}</p>}
        {check !== null && check.findings.length > 0 && (
          <ul className="space-y-3" aria-label="Launch check findings">
            {orderFindings(check.findings).map((finding, index) => (
              <FindingCard key={`${finding.kind}-${index}`} finding={finding} />
            ))}
          </ul>
        )}
      </section>

      <section aria-labelledby="deployment-heading-overview" className="space-y-3">
        <h3 id="deployment-heading-overview" className="text-lg font-semibold">How this instance runs</h3>
        <div className="flex flex-wrap gap-2" role="group" aria-label="Deployment">
          {DEPLOYMENT_OPTIONS.map((option) => {
            const chosen = option.id === choice;
            return (
              <button
                key={option.id}
                type="button"
                aria-pressed={chosen}
                disabled={busy !== null || detail.running}
                onClick={() => void changeDeployment(option.id)}
                className={[
                  'rounded-lg px-3 py-1.5 text-sm font-medium transition-colors disabled:opacity-50',
                  chosen
                    ? 'bg-primary text-primary-foreground shadow-sm'
                    : 'border border-border text-muted-foreground hover:bg-accent hover:text-accent-foreground',
                ].join(' ')}
              >
                {option.label}
              </button>
            );
          })}
        </div>
        <p className="text-sm text-muted-foreground" data-testid="deployment-explained">
          <span className="font-medium text-foreground">{deploymentLabel(detail.deployment)}.</span>{' '}
          {choiceCopy?.explain}
        </p>
        {detail.running && (
          <p className="text-xs text-muted-foreground">The way the instance runs can be changed once the game has closed.</p>
        )}
      </section>

      <section aria-labelledby="launch-heading" className="space-y-4 rounded-xl border border-border bg-card p-5">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h3 id="launch-heading" className="text-lg font-semibold">Launch</h3>
            <p className="text-sm text-muted-foreground">Last played: {formatWhen(detail.lastLaunchedAt)}</p>
          </div>
          <Button
            type="button"
            size="lg"
            onClick={() => void launch()}
            disabled={busy !== null || detail.running}
          >
            <Play aria-hidden="true" />
            Launch {detail.name}
          </Button>
        </div>

        {busy === 'launch' && (
          <div role="status" className="space-y-1 text-sm">
            <p className="font-medium">Checking, preparing and starting the game…</p>
            <p className="text-muted-foreground">
              {elapsedText(elapsed)} so far. A large instance can take several minutes the first time, and Agora cannot show a percentage yet.
            </p>
          </div>
        )}

        {reply !== null && <LaunchResult reply={reply} onChoose={(next, always) => void answerFallback(next, always)} onDismiss={() => setReply(null)} />}

        <div className="border-t border-border pt-3">
          <button
            type="button"
            aria-expanded={advancedOpen}
            onClick={() => setAdvancedOpen((open) => !open)}
            className="text-sm font-medium text-muted-foreground underline-offset-4 hover:underline"
          >
            {advancedOpen ? 'Hide advanced options' : 'Advanced options'}
          </button>
          {advancedOpen && (
            <div className="mt-3 space-y-2 rounded-lg border border-destructive/40 p-3">
              <label className="flex items-start gap-2 text-sm">
                <input
                  type="checkbox"
                  className="mt-1"
                  checked={launchAnyway}
                  onChange={(event) => setLaunchAnyway(event.target.checked)}
                />
                <span>
                  <span className="font-medium">Launch anyway</span>: start the game past the checks above, even when they would stop it.
                </span>
              </label>
              <p className="text-xs text-destructive">
                The game may crash or behave in the ways these checks exist to prevent. Use this only when you know why a check is wrong. Agora does not undo a session that goes badly.
              </p>
            </div>
          )}
        </div>
      </section>
    </section>
  );
}

function FindingCard({ finding }: { finding: CheckFinding }) {
  return (
    <li className="space-y-1 rounded-lg border border-border bg-card p-4" data-testid="check-finding" data-blocks={finding.blocks ? 'true' : 'false'}>
      <div className="flex flex-wrap items-center gap-2">
        <span
          className={[
            'rounded-full px-2 py-0.5 text-xs font-semibold',
            finding.blocks ? 'bg-destructive text-destructive-foreground' : 'border border-border text-muted-foreground',
          ].join(' ')}
        >
          {finding.blocks ? 'Stops the launch' : 'To review'}
        </span>
        <span className="font-semibold">{finding.title}</span>
      </div>
      <p className="text-sm">{finding.detail}</p>
      <p className="text-sm text-muted-foreground">
        <span className="font-medium text-foreground">Why it matters: </span>
        {finding.why}
      </p>
      {finding.repair !== null && (
        <p className="text-sm">
          <span className="font-medium">What to do: </span>
          {finding.repair}
        </p>
      )}
    </li>
  );
}

function LaunchResult({
  reply,
  onChoose,
  onDismiss,
}: {
  reply: LaunchReply;
  onChoose: (next: DeploymentMode, always: boolean) => void;
  onDismiss: () => void;
}) {
  if (reply.status === 'launched') {
    return (
      <div role="status" className="space-y-2 rounded-lg border border-border p-3 text-sm">
        <p className="font-medium">The game is starting (process {reply.pid}). Agora will put your files back when it closes.</p>
        <p className="text-muted-foreground">
          {reply.deployment === null
            ? 'Running from the base, with nothing deployed on top of it.'
            : `Running from ${deploymentLabel(reply.deployment).toLowerCase()}.`}
        </p>
        {reply.notice !== null && <p>{reply.notice}</p>}
        {reply.findings.length > 0 && (
          <div className="space-y-1">
            <p className="font-medium">Launched past these:</p>
            <ul className="list-disc pl-5">
              {reply.findings.map((finding, index) => (
                <li key={index}>{finding.title}: {finding.detail}</li>
              ))}
            </ul>
          </div>
        )}
      </div>
    );
  }

  if (reply.status === 'refused') {
    return (
      <div role="alert" className="space-y-2 rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm">
        <p className="font-medium">The launch was stopped by these checks. Nothing was started.</p>
        <p>Fix them above, or turn on Advanced options and launch anyway at your own risk.</p>
        <button type="button" className="text-xs underline" onClick={onDismiss}>Dismiss</button>
      </div>
    );
  }

  // The virtual file system would not start. The user decides; nothing was started.
  const next = reply.next;
  return (
    <div role="alertdialog" aria-label="The virtual file system could not start" className="space-y-3 rounded-lg border border-border p-4 text-sm" data-testid="vfs-question">
      <p className="font-semibold">The virtual file system could not start.</p>
      {reply.reason !== null && <p className="text-muted-foreground">{reply.reason}</p>}
      {next === null ? (
        <p>There is no simpler way to run this instance. Fix the reason above, then try again.</p>
      ) : (
        <>
          <p>Agora can run this instance from {deploymentLabel(next).toLowerCase()} instead. Nothing has been changed yet.</p>
          <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
            {fallbackTradeoffs(next).map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
          <div className="flex flex-wrap gap-2">
            <Button type="button" onClick={() => onChoose(next, false)}>
              Run once from {deploymentLabel(next).toLowerCase()}
            </Button>
            <Button type="button" variant="outline" onClick={() => onChoose(next, true)}>
              Always run from {deploymentLabel(next).toLowerCase()}
            </Button>
          </div>
        </>
      )}
      <button type="button" className="text-xs underline" onClick={onDismiss}>Cancel</button>
    </div>
  );
}
