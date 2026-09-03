import { useCallback, useEffect, useRef, useState } from 'react';
import { ModBisectPanel } from './ModBisectPanel';
import { invoke } from '@tauri-apps/api/core';
import ReactMarkdown from 'react-markdown';
import {
  confirmCrashFix,
  createSnapshot,
  deleteSnapshot,
  disableModForTest,
  formatError,
  getDisablePlan,
  getSetting,
  investigateCrash,
  investigateInstanceEvidence,
  investigateManual,
  pickAndInvestigateCrashEvidence,
  readCrashLog,
  reportStillCrashing,
  restoreSnapshot,
  type DisablePlan,
  type CrashInvestigation,
  type InvestigationResult,
  type SuspectScore,
  type SuggestedAction,
} from '../lib/tauri';
import { DependencyPrompt } from './DependencyPrompt';
import { AiAssistant } from './AiAssistant';
import { tryEarnInteraction } from '../features/interactive/live/interactionAchievements';
import type { ProcessState } from '../lib/useProcessController';
import type { LaunchStartOutcome } from '../lib/useProcessController';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog';

interface CrashInvestigatorProps {
  instanceId: string;
  crashFilename?: string | null;
  manualLogText?: string | null;
  onClose: () => void;
  /** Called to re-launch the instance after disabling a suspected mod. */
  /** Returns true only when the canonical launch controller actually started. */
  onLaunch: (onAwaitingHealth: () => void) => Promise<LaunchStartOutcome>;
  processState: ProcessState;
}

function investigationResultFromEvidence(investigation: CrashInvestigation): InvestigationResult {
  const top = investigation.suspects[0];
  return {
    fingerprint: investigation.fingerprint,
    signature_name: investigation.triage.signature_name,
    suspects: investigation.suspects,
    suggested_action: top
      ? { kind: 'GuidedDisable', next_suspect: top }
      : { kind: 'NoSuspects' },
    ruled_out: [],
  };
}

function combinedEvidenceText(investigation: CrashInvestigation): string {
  return investigation.evidence.sources
    .map((source) => `===== ${source.meta.basename} =====\n${source.text}`)
    .join('\n\n');
}

/** Render a single suspect card. */
function SuspectCard({
  suspect,
  rank,
  isTop,
  action,
  onAction,
  loading,
}: {
  suspect: SuspectScore;
  rank: number;
  isTop: boolean;
  action?: SuggestedAction;
  onAction?: () => void;
  loading: boolean;
}) {
  const score = suspect.total_score.toFixed(2);
  return (
    <div
      className={[
        'rounded-xl border p-4 transition-colors',
        isTop
          ? 'border-primary/30 bg-primary/10'
          : 'border-border bg-card',
      ].join(' ')}
    >
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-3 min-w-0">
          <span
            className={[
              'inline-flex h-6 w-6 items-center justify-center rounded-full text-xs font-bold shrink-0',
              isTop
                ? 'bg-primary text-primary-foreground'
                : 'bg-card text-muted-foreground',
            ].join(' ')}
          >
            {rank}
          </span>
          <div className="min-w-0">
            <p className="text-sm font-semibold break-all">{suspect.filename}</p>
            {suspect.mod_id && suspect.mod_id !== suspect.filename && (
              <p className="text-xs text-muted-foreground break-all">{suspect.mod_id}</p>
            )}
            {suspect.is_dependent_of && (
              <span className="mt-1 inline-block rounded-md bg-amber-100 dark:bg-amber-900/30 px-2 py-0.5 text-xs font-medium text-amber-800 dark:text-amber-300">
                Needs {suspect.is_dependent_of} to work — they’ll be turned off together
              </span>
            )}
            {isTop && (
              <p className="mt-1 text-xs text-muted-foreground">
                Looks like the most likely cause — try the test below.
              </p>
            )}
          </div>
        </div>
        <span className="font-mono text-sm font-bold text-muted-foreground shrink-0">
          {score}
        </span>
      </div>
      {isTop && action && (
        <div className="mt-3 pt-3 border-t border-primary/20">
          {action.kind === 'GuidedDisable' && (
            <button
              disabled={loading}
              onClick={onAction}
              className={[
                'w-full rounded-lg px-3 py-2 text-sm font-medium transition-colors',
                'bg-primary text-primary-foreground hover:bg-primary/90',
                'disabled:opacity-50 disabled:cursor-not-allowed',
              ].join(' ')}
            >
              Try without &quot;{suspect.filename}&quot;
            </button>
          )}
          {action.kind === 'ConfidenceAutoDisable' && (
            <button
              disabled={loading}
              onClick={onAction}
              className={[
                'w-full rounded-lg px-3 py-2 text-sm font-medium transition-colors',
                'bg-primary text-primary-foreground hover:bg-primary/90',
                'disabled:opacity-50 disabled:cursor-not-allowed',
              ].join(' ')}
            >
              Try without &quot;{action.mod_id}&quot;
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/** Post-launch confirmation prompt. */
function FixConfirmation({
  filename,
  onFix,
  onStillCrashing,
  loading,
}: {
  filename: string;
  onFix: () => void;
  onStillCrashing: () => void;
  loading: boolean;
}) {
  return (
    <div className="rounded-xl border border-primary/30 bg-primary/10 p-4">
      <p className="text-sm font-semibold mb-3">
        Did the game start properly without &quot;{filename}&quot;?
      </p>
      <p className="text-xs text-muted-foreground mb-3">
        If the game opened and you could play, choose “Yes.” If it crashed again, choose “No.”
      </p>
      <div className="flex gap-2">
        <button
          disabled={loading}
          onClick={onFix}
          className={[
            'flex-1 rounded-lg px-3 py-2 text-sm font-medium transition-colors',
            'bg-green-600 text-white hover:bg-green-700',
            'disabled:opacity-50 disabled:cursor-not-allowed',
          ].join(' ')}
        >
          Yes, it worked
        </button>
        <button
          disabled={loading}
          onClick={onStillCrashing}
          className={[
            'flex-1 rounded-lg px-3 py-2 text-sm font-medium transition-colors',
            'bg-destructive text-destructive-foreground hover:bg-destructive/90',
            'disabled:opacity-50 disabled:cursor-not-allowed',
          ].join(' ')}
        >
          No, still crashing
        </button>
      </div>
    </div>
  );
}

/** Triage banner for mods under community review. */
function TriageBanner({ modId, onViewTriage }: { modId: string; onViewTriage: () => void }) {
  return (
    <div className="rounded-xl border border-yellow-300 dark:border-yellow-700 bg-yellow-50 dark:bg-yellow-900/20 p-4">
      <p className="text-sm text-yellow-800 dark:text-yellow-200 mb-3">
        Other players have reported problems with “{modId}” — our moderators are looking into it.
      </p>
      <button
        onClick={onViewTriage}
        className="rounded-lg px-3 py-2 text-sm font-medium transition-colors bg-yellow-600 text-white hover:bg-yellow-700"
      >
        View in Triage Center
      </button>
    </div>
  );
}

/** Success confirmation overlay. */
function SuccessBanner({ message }: { message: string }) {
  return (
    <div className="rounded-xl border border-green-300 dark:border-green-700 bg-green-50 dark:bg-green-900/20 p-4">
      <p className="text-sm font-semibold text-green-800 dark:text-green-200">
        {message}
      </p>
    </div>
  );
}

/** Error banner. */
function ErrorBanner({ message }: { message: string }) {
  return (
    <div className="rounded-xl border border-destructive/30 bg-destructive/10 p-4">
      <p className="text-sm font-semibold text-destructive">
        {message}
      </p>
    </div>
  );
}

/** Ruled-out mods list. */
function RuledOutList({ ruledOut }: { ruledOut: string[] }) {
  if (ruledOut.length === 0) return null;
  return (
    <div className="mt-2 text-xs text-muted-foreground">
      We already tried without:{' '}
      <span className="font-medium">{ruledOut.join(', ')}</span>
    </div>
  );
}

export function CrashInvestigator({
  instanceId,
  crashFilename,
  manualLogText,
  onClose,
  onLaunch,
  processState,
}: CrashInvestigatorProps) {
  const [result, setResult] = useState<InvestigationResult | null>(null);
  const [evidence, setEvidence] = useState<CrashInvestigation | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Raw crash log text, stored for reportStillCrashing in file-mode investigations
  const [crashLogText, setCrashLogText] = useState<string>('');
  // Post-launch state
  const [postLaunch, setPostLaunch] = useState<{
    filename: string;
    modId: string;
  } | null>(null);
  const [pendingTest, setPendingTest] = useState<{
    filename: string;
    modId: string;
    armed: boolean;
  } | null>(null);
  const [experimentNotice, setExperimentNotice] = useState<string | null>(null);
  // Success state
  const [success, setSuccess] = useState<string | null>(null);
  const [recoverySnapshotId, setRecoverySnapshotId] = useState<string | null>(null);
  const recoverySnapshotIdRef = useRef<string | null>(null);
  const [disabledByTest, setDisabledByTest] = useState<string[]>([]);
  // Disable dependency prompt state
  const [disablePlanTarget, setDisablePlanTarget] = useState<{
    originalFilename: string;
    modId: string;
    plan: DisablePlan;
  } | null>(null);
  // AI assistant panel
  const [showAiAssistant, setShowAiAssistant] = useState(false);
  // AI crash explanation
  const [aiExplanation, setAiExplanation] = useState<string | null>(null);
  const [aiLoading, setAiLoading] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const [showPaste, setShowPaste] = useState(false);
  const [pasteText, setPasteText] = useState('');
  const [aiChatEnabled, setAiChatEnabled] = useState(false);
  const cancelledRef = useRef(false);
  const closeInProgressRef = useRef(false);
  const pendingLaunchRef = useRef(false);
  const stackedHealthDialogRef = useRef(false);

  // Run investigation on mount
  useEffect(() => {
    let cancelled = false;
    const runInvestigation = async () => {
      try {
        // For file-based investigation, fetch the raw log text first
        if (crashFilename) {
          const rawText = await readCrashLog(instanceId, crashFilename);
          if (!cancelled) setCrashLogText(rawText);
        }

        let invResult: InvestigationResult;
        if (manualLogText) {
          invResult = await investigateManual(instanceId, manualLogText);
        } else if (crashFilename) {
          invResult = await investigateCrash(instanceId, crashFilename || undefined);
        } else {
          const automatic = await investigateInstanceEvidence(instanceId);
          if (cancelled) return;
          setEvidence(automatic);
          setCrashLogText(combinedEvidenceText(automatic));
          invResult = investigationResultFromEvidence(automatic);
        }
        if (!cancelled) setResult(invResult);
      } catch (e) {
        if (!cancelled) {
          setError(formatError(e));
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    runInvestigation();
    return () => {
      cancelled = true;
    };
  }, [instanceId, crashFilename, manualLogText]);

  const ensureRecoverySnapshot = useCallback(async () => {
    if (recoverySnapshotIdRef.current) return recoverySnapshotIdRef.current;
    const snapshot = await createSnapshot(
      instanceId,
      `crash-doctor-${globalThis.crypto?.randomUUID?.() ?? Date.now()}`,
    );
    recoverySnapshotIdRef.current = snapshot.id;
    setRecoverySnapshotId(snapshot.id);
    return snapshot.id;
  }, [instanceId]);

  const clearRecoverySnapshotState = useCallback(() => {
    recoverySnapshotIdRef.current = null;
    setRecoverySnapshotId(null);
    setDisabledByTest([]);
    setPostLaunch(null);
  }, []);

  const discardRecoverySnapshot = useCallback(async () => {
    const snapshotId = recoverySnapshotIdRef.current;
    clearRecoverySnapshotState();
    if (!snapshotId) return;

    // The recovery point is protected from normal retention while this
    // component owns it. Cleanup is best-effort after the experiment ends.
    try {
      await deleteSnapshot(instanceId, snapshotId);
    } catch (cause) {
      const message = formatError(cause).toLowerCase();
      if (!message.includes('not found') && !message.includes('not_found')) {
        console.warn('Crash Doctor could not remove its completed recovery point', cause);
      }
    }
  }, [clearRecoverySnapshotState, instanceId]);

  const restoreInvestigationSnapshot = useCallback(async () => {
    const snapshotId = recoverySnapshotIdRef.current;
    if (!snapshotId) {
      throw new Error('We don’t have a backup to put back — nothing to undo.');
    }
    await restoreSnapshot(instanceId, snapshotId);
    setDisabledByTest([]);
    setPostLaunch(null);
  }, [instanceId]);

  const handleClose = useCallback(async () => {
    if (closeInProgressRef.current) return;
    closeInProgressRef.current = true;
    if (success) {
      onClose();
      closeInProgressRef.current = false;
      return;
    }
    if (!recoverySnapshotId && disabledByTest.length === 0) {
      onClose();
      closeInProgressRef.current = false;
      return;
    }
    setLoading(true);
    setError(null);
    try {
      await restoreInvestigationSnapshot();
      await discardRecoverySnapshot();
      onClose();
    } catch (cause) {
      const raw = formatError(cause);
      const lower = raw.toLowerCase();
      const isMissing = lower.includes('not found') || lower.includes('not_found') || lower.includes('unavailable') || lower.includes('no backup');
      if (isMissing) {
        // Backup is gone — nothing to undo, so let the user close safely.
        clearRecoverySnapshotState();
        if (!cancelledRef.current) {
          setError(null);
          setLoading(false);
        }
        onClose();
        closeInProgressRef.current = false;
        return;
      }
      setError(`We couldn't put your mods back the way they were: ${raw}. You can try again, or close and keep things as they are — nothing will be lost.`);
    } finally {
      if (!cancelledRef.current) setLoading(false);
      closeInProgressRef.current = false;
    }
  }, [clearRecoverySnapshotState, disabledByTest.length, discardRecoverySnapshot, onClose, recoverySnapshotId, restoreInvestigationSnapshot, success]);

  const handleDialogOpenChange = useCallback((open: boolean) => {
    if (open) return;
    // Radix closes the underlying modal when Health Check is stacked above it.
    // Keep Crash Doctor mounted so approval retains the guided experiment.
    if (
      pendingLaunchRef.current
      || stackedHealthDialogRef.current
      || (pendingTest && processState.healthReport)
    ) return;
    void handleClose();
  }, [handleClose, pendingTest, processState.healthReport]);

  const handleDisableAndRelaunch = useCallback(async () => {
    if (!result) return;
    setLoading(true);
    setError(null);

    let action: SuggestedAction;
    let filename: string;
    let modId: string;

    if (result.suggested_action.kind === 'GuidedDisable') {
      action = result.suggested_action;
      filename = action.next_suspect.filename;
      modId = action.next_suspect.mod_id;
    } else if (result.suggested_action.kind === 'ConfidenceAutoDisable') {
      action = result.suggested_action;
      filename = action.filename;
      modId = action.mod_id;
    } else {
      setError('We didn’t find anything to try turning off right now.');
      setLoading(false);
      return;
    }

    // Detective achievement: picking a crash suspect (counts even if dependents prompt follows)
    tryEarnInteraction('suspect');

    let modified = false;
    try {
      const plan = await getDisablePlan(instanceId, filename);
      if (plan.dependents.length > 0) {
        setDisablePlanTarget({ originalFilename: filename, modId, plan });
        setLoading(false);
        return;
      }
      await ensureRecoverySnapshot();
      await disableModForTest(instanceId, filename);
      modified = true;
      setDisabledByTest([filename]);
      pendingLaunchRef.current = true;
      const launchOutcome = await onLaunch(() => {
        stackedHealthDialogRef.current = true;
        setPendingTest({ filename, modId, armed: true });
      });
      if (!cancelledRef.current && launchOutcome !== 'failed') {
        setPendingTest({ filename, modId, armed: true });
      } else if (!cancelledRef.current) {
        pendingLaunchRef.current = false;
        setPendingTest(null);
        await restoreInvestigationSnapshot();
        setError('The game didn’t start for the test. Check the message shown, fix it, then try again.');
      }
    } catch (e) {
      if (modified) {
        try {
          await restoreInvestigationSnapshot();
        } catch (restoreError) {
          if (!cancelledRef.current) {
            setError(`We tried to test “${filename}” but something went wrong and we couldn’t put your mods back: ${formatError(restoreError)}. You can close and keep things as they are.`);
          }
          return;
        }
      }
      if (!cancelledRef.current) {
        setError(formatError(e));
      }
    } finally {
      if (!cancelledRef.current && !disablePlanTarget) setLoading(false);
    }
  }, [result, instanceId, onLaunch, restoreInvestigationSnapshot, ensureRecoverySnapshot]);

  const handleDisableConfirm = useCallback(async (selectedKeys: string[]) => {
    if (!disablePlanTarget) return;
    const { originalFilename, modId, plan } = disablePlanTarget;
    setLoading(true);
    setError(null);

    try {
      await ensureRecoverySnapshot();
      const selectedSet = new Set(selectedKeys);
      const filenames = [
        ...plan.dependents
          .filter((dependent) => selectedSet.has(dependent.mod_id))
          .map((dependent) => dependent.filename),
        originalFilename,
      ];
      for (const filename of filenames) {
        await disableModForTest(instanceId, filename);
      }
      setDisabledByTest(filenames);
      pendingLaunchRef.current = true;
      const launchOutcome = await onLaunch(() => {
        stackedHealthDialogRef.current = true;
        setPendingTest({ filename: originalFilename, modId, armed: true });
      });
      if (!cancelledRef.current && launchOutcome !== 'failed') {
        setPendingTest({ filename: originalFilename, modId, armed: true });
      } else if (!cancelledRef.current) {
        pendingLaunchRef.current = false;
        setPendingTest(null);
        await restoreInvestigationSnapshot();
        setError('The game didn’t start for the test. Check the message shown, fix it, then try again.');
      }
    } catch (e) {
      try {
        await restoreInvestigationSnapshot();
      } catch (restoreError) {
        if (!cancelledRef.current) {
          setError(`We tried to turn off “${originalFilename}” but something went wrong and we couldn’t put your mods back: ${formatError(restoreError)}. You can close and keep things as they are.`);
        }
        return;
      }
      if (!cancelledRef.current) {
        setError(formatError(e));
      }
    } finally {
      if (!cancelledRef.current) {
        setDisablePlanTarget(null);
        setLoading(false);
      }
    }
  }, [disablePlanTarget, instanceId, onLaunch, restoreInvestigationSnapshot, ensureRecoverySnapshot]);

  const handleAddEvidence = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const investigation = await pickAndInvestigateCrashEvidence(instanceId);
      if (!investigation || cancelledRef.current) return;
      setEvidence(investigation);
      setCrashLogText(combinedEvidenceText(investigation));
      setResult(investigationResultFromEvidence(investigation));
    } catch (cause) {
      if (!cancelledRef.current) setError(formatError(cause));
    } finally {
      if (!cancelledRef.current) setLoading(false);
    }
  }, [instanceId]);

  const handlePasteEvidence = useCallback(async () => {
    if (!pasteText.trim()) return;
    setLoading(true);
    setError(null);
    try {
      const investigation = await investigateManual(instanceId, pasteText);
      if (!cancelledRef.current) {
        setEvidence(null);
        setCrashLogText(pasteText);
        setResult(investigation);
        setShowPaste(false);
      }
    } catch (cause) {
      if (!cancelledRef.current) setError(formatError(cause));
    } finally {
      if (!cancelledRef.current) setLoading(false);
    }
  }, [instanceId, pasteText]);

  // Respect the AI chat setting for both AI entry points.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const v = await getSetting('ai_chat_enabled');
        if (!cancelled) setAiChatEnabled(v === true || v === 'true');
      } catch {
        if (!cancelled) setAiChatEnabled(false);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    if (!aiChatEnabled) setShowAiAssistant(false);
  }, [aiChatEnabled]);

  // Track whether the component is still mounted.
  // Reset on setup so StrictMode double-invocation (dev) or real remounts
  // don't leave cancelledRef stuck at true from a previous cleanup.
  useEffect(() => {
    cancelledRef.current = false;
    return () => {
      cancelledRef.current = true;
    };
  }, []);

  const handleFixConfirmed = useCallback(async () => {
    if (!result || !postLaunch) return;
    setLoading(true);
    setError(null);

    try {
      if (result.fingerprint) {
        await confirmCrashFix(result.fingerprint, postLaunch.modId);
      }
      if (!cancelledRef.current) {
        await discardRecoverySnapshot();
        setSuccess(`Got it — keeping “${postLaunch.modId}” turned off fixed the crash. You can turn it back on later from your mods list.`);
        // Auto-close after a short delay
        setTimeout(() => {
          if (!cancelledRef.current) {
            onClose();
          }
        }, 2000);
      }
    } catch (e) {
      if (!cancelledRef.current) {
        setError(formatError(e));
      }
    } finally {
      if (!cancelledRef.current) setLoading(false);
    }
  }, [discardRecoverySnapshot, result, postLaunch, onClose]);

  const handleStillCrashing = useCallback(async () => {
    if (!result || !postLaunch) return;
    setLoading(true);
    setError(null);

    try {
      // Restore the complete pre-investigation state, including any dependents.
      await restoreInvestigationSnapshot();

      // Determine the crash log text to pass
      let logText: string;
      if (manualLogText) {
        logText = manualLogText;
      } else {
        // File mode: re-fetch the raw log text (we may have it in state already)
        logText = crashLogText || '';
        if (!logText) {
          logText = await readCrashLog(instanceId, crashFilename || '');
          setCrashLogText(logText);
        }
      }

      // reportStillCrashing returns a new InvestigationResult (auto-advance)
      const newResult = await reportStillCrashing(
        instanceId,
        result.fingerprint!,
        postLaunch.modId,
        logText,
      );

      if (!cancelledRef.current) {
        setResult(newResult);
        setPostLaunch(null);
      }
    } catch (e) {
      if (!cancelledRef.current) {
        setError(formatError(e));
      }
    } finally {
      if (!cancelledRef.current) setLoading(false);
    }
  }, [result, postLaunch, instanceId, manualLogText, crashLogText, crashFilename, restoreInvestigationSnapshot]);

  useEffect(() => {
    if (!pendingTest?.armed) return;
    let cancelled = false;
    const finishTest = async () => {
      const failedBeforeStart = (
        processState.phase === 'failed'
        && processState.instanceId === instanceId
        && !processState.healthReport
      ) || (
        processState.phase === 'idle'
        && processState.instanceId === null
      );
      if (failedBeforeStart) {
        try {
          await restoreInvestigationSnapshot();
          if (!cancelled) {
            pendingLaunchRef.current = false;
            globalThis.setTimeout(() => {
              stackedHealthDialogRef.current = false;
            }, 250);
            setPendingTest(null);
            setError(processState.error ?? 'The game didn’t start for the test. We put your mods back how they were.');
          }
        } catch (cause) {
          if (!cancelled) setError(`The test didn’t start and we couldn’t put your mods back: ${formatError(cause)}. You can close and keep things as they are.`);
        }
        return;
      }

      if (processState.instanceId !== instanceId || processState.phase !== 'exited') return;
      if (processState.outcome === 'success') {
        if (!cancelled) {
          pendingLaunchRef.current = false;
          stackedHealthDialogRef.current = false;
          setPostLaunch(pendingTest);
          setPendingTest(null);
        }
        return;
      }

      if (processState.outcome === 'crash') {
        setLoading(true);
        try {
          await restoreInvestigationSnapshot();
          const latest = await investigateInstanceEvidence(instanceId);
          const sameFingerprint = result?.fingerprint
            && latest.fingerprint
            && result.fingerprint.exception_class === latest.fingerprint.exception_class
            && result.fingerprint.top_frames.join('\n') === latest.fingerprint.top_frames.join('\n');
          const latestText = combinedEvidenceText(latest);
          if (sameFingerprint && result?.fingerprint) {
            const advanced = await reportStillCrashing(
              instanceId,
              result.fingerprint,
              pendingTest.modId,
              latestText,
            );
            if (!cancelled) {
              setResult(advanced);
              setExperimentNotice(`It still crashed without “${pendingTest.filename}”, so that’s probably not the cause. We turned it back on.`);
            }
          } else if (!cancelled) {
            setEvidence(latest);
            setCrashLogText(latestText);
            setResult(investigationResultFromEvidence(latest));
            setExperimentNotice('Something different happened this time, so we didn’t learn for sure. We put your mods back how they were.');
          }
        } catch (cause) {
          if (!cancelled) setError(`We couldn’t check the test result: ${formatError(cause)}. Your mods were put back.`);
        } finally {
          if (!cancelled) {
            pendingLaunchRef.current = false;
            stackedHealthDialogRef.current = false;
            setPendingTest(null);
            setLoading(false);
          }
        }
        return;
      }

      try {
        await restoreInvestigationSnapshot();
        if (!cancelled) {
          pendingLaunchRef.current = false;
          stackedHealthDialogRef.current = false;
          setExperimentNotice('We couldn’t tell if that helped — the game didn’t finish starting. We put your mods back.');
          setPendingTest(null);
        }
      } catch (cause) {
        if (!cancelled) setError(`We couldn’t put your mods back after the test: ${formatError(cause)}. You can close and keep things as they are.`);
      }
    };
    void finishTest();
    return () => { cancelled = true; };
  }, [instanceId, pendingTest, processState.error, processState.healthReport, processState.instanceId, processState.outcome, processState.phase, restoreInvestigationSnapshot, result]);

  const handleViewTriage = useCallback(() => {
    onClose();
  }, [onClose]);

  const handleAiExplain = useCallback(async () => {
    if (!aiChatEnabled || aiLoading || cancelledRef.current) return;
    setAiLoading(true);
    setAiError(null);
    setAiExplanation(null);
    const logText = crashLogText || manualLogText || '';
    if (!logText) {
      setAiError('No crash log available to analyze.');
      setAiLoading(false);
      return;
    }
    try {
      const explanation = await invoke<string>('explain_crash', {
        instanceId: instanceId,
        crashLog: logText,
      });
      if (!cancelledRef.current) setAiExplanation(explanation);
    } catch (e) {
      const msg = formatError(e);
      if (msg.includes('ERR_AI_NOT_AUTHENTICATED') || msg.toLowerCase().includes('not authenticated') || msg.toLowerCase().includes('not connected')) {
        if (!cancelledRef.current) setAiError('connect-github');
      } else {
        if (!cancelledRef.current) setAiError(msg);
      }
    } finally {
      if (!cancelledRef.current) setAiLoading(false);
    }
  }, [instanceId, crashLogText, manualLogText, aiLoading, aiChatEnabled]);

  if (loading && !result) {
    return (
      <Dialog open onOpenChange={handleDialogOpenChange}>
        <DialogContent className="max-w-5xl">
          <DialogTitle>Crash Doctor</DialogTitle>
          <DialogDescription>
            Checking your recent crash reports and game logs to figure out what went wrong.
          </DialogDescription>
          <div className="flex flex-col items-center gap-3 py-8">
            <div role="status" aria-label="Looking at your crash" className="h-8 w-8 animate-spin rounded-full border-2 border-primary border-t-transparent" />
            <p className="text-sm text-muted-foreground">Looking at your crash…</p>
          </div>
        </DialogContent>
      </Dialog>
    );
  }

  if (error) {
    const lowerErr = error.toLowerCase();
    const isMissingBackup = lowerErr.includes('not found') || lowerErr.includes('not_found') || lowerErr.includes('no backup') || lowerErr.includes('unavailable');
    const friendlyMessage = isMissingBackup
      ? 'We couldn’t find the backup we made before testing, so there’s nothing to put back. Your current mods will stay as they are — you can safely close.'
      : error.startsWith('We couldn')
        ? error
        : `Something didn’t work: ${error}. Your game files are still safe.`;
    return (
      <Dialog open onOpenChange={handleDialogOpenChange}>
        <DialogContent className="max-w-5xl">
          <DialogTitle>Crash Doctor</DialogTitle>
          <DialogDescription>
            {isMissingBackup
              ? 'There’s no backup to restore, so you can close without worry.'
              : 'We hit a hiccup, but your files are safe. Choose whether to put your mods back how they were or keep them as they are.'}
          </DialogDescription>
          <ErrorBanner message={friendlyMessage} />
          <div className="flex justify-end gap-2 flex-wrap">
            {recoverySnapshotId && !isMissingBackup && (
              <button
                onClick={() => {
                  void restoreInvestigationSnapshot()
                    .then(() => discardRecoverySnapshot())
                    .then(onClose)
                    .catch((cause) => {
                      const msg = formatError(cause);
                      const l = msg.toLowerCase();
                      if (l.includes('not found') || l.includes('not_found') || l.includes('unavailable')) {
                        clearRecoverySnapshotState();
                        onClose();
                      } else {
                        setError(`We couldn't put your mods back: ${msg}. You can try again or close and keep things as they are.`);
                      }
                    });
                }}
                className="rounded-lg border border-border px-4 py-2 text-sm font-medium hover:bg-accent"
              >
                Put mods back and close
              </button>
            )}
            <button
              onClick={() => {
                // Always allow closing without trying to restore again — this breaks the loop.
                setError(null);
                void discardRecoverySnapshot().finally(onClose);
              }}
              className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
            >
              {isMissingBackup ? 'Close' : 'Keep as is and close'}
            </button>
          </div>
        </DialogContent>
      </Dialog>
    );
  }

  if (!result) return null;

  const { fingerprint, signature_name, suspects, suggested_action, ruled_out } = result;

  // Determine the action card for the top suspect
  let actionCard: SuggestedAction | undefined;
  if (suggested_action.kind === 'GuidedDisable') {
    actionCard = suggested_action;
  } else if (suggested_action.kind === 'ConfidenceAutoDisable') {
    actionCard = suggested_action;
  }

  return (
    <>
      <Dialog open onOpenChange={handleDialogOpenChange}>
        <DialogContent className="max-h-[90vh] max-w-5xl overflow-hidden flex flex-col">
          <div className="flex items-start justify-between gap-4 border-b border-border pb-4 pr-6 shrink-0">
          <div className="flex-1 min-w-0">
            <DialogTitle>Crash Doctor</DialogTitle>
            <DialogDescription>
              We look for clues in your logs and test likely causes one at a time. Anything we change can be undone.
            </DialogDescription>
            {fingerprint && (
              <p className="text-sm text-muted-foreground mt-1 truncate" title={fingerprint.exception_class}>
                Error: {fingerprint.exception_class}
              </p>
            )}
            {signature_name && (
              <p className="text-xs text-primary mt-0.5">
                {signature_name}
              </p>
            )}
          </div>
          {aiChatEnabled && (
            <div className="flex items-center gap-2">
              <button
                onClick={() => setShowAiAssistant(true)}
                className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90"
              >
                Ask AI Assistant
              </button>
            </div>
          )}
        </div>

        <div className="flex-1 overflow-y-auto space-y-4 pr-1">
          <div className="rounded-lg border border-border bg-muted/40 p-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div>
                <p className="text-sm font-semibold">What we checked</p>
                <p className="text-xs text-muted-foreground">
                  {evidence
                    ? `We checked ${evidence.evidence.sources.length} file${evidence.evidence.sources.length === 1 ? '' : 's'} from your game folder — nothing was sent online`
                    : manualLogText || crashFilename
                      ? 'Using the crash info you shared'
                      : 'No recent crash files were found'}
                </p>
              </div>
              <div className="flex gap-2">
                <button
                  onClick={() => { void handleAddEvidence(); }}
                  disabled={loading}
                  className="rounded-md border border-input bg-background px-2.5 py-1.5 text-xs font-medium hover:bg-accent disabled:opacity-50"
                >
                  Choose a file
                </button>
                <button
                  onClick={() => setShowPaste((value) => !value)}
                  disabled={loading}
                  className="rounded-md border border-input bg-background px-2.5 py-1.5 text-xs font-medium hover:bg-accent disabled:opacity-50"
                >
                  Paste crash text
                </button>
              </div>
            </div>
            {showPaste && (
              <div className="mt-3 space-y-2">
                <textarea
                  value={pasteText}
                  onChange={(event) => setPasteText(event.target.value)}
                  placeholder="Paste your crash report here"
                  className="h-32 w-full resize-y rounded-md border border-input bg-background p-2 font-mono text-xs"
                />
                <div className="flex justify-end">
                  <button
                    onClick={() => { void handlePasteEvidence(); }}
                    disabled={!pasteText.trim() || loading}
                    className="rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground disabled:opacity-50"
                  >
                    Check this text
                  </button>
                </div>
              </div>
            )}
            {evidence && evidence.evidence.sources.length > 0 && (
              <div className="mt-3 space-y-2">
                {evidence.evidence.sources.map((source, index) => (
                  <details key={`${source.meta.kind}:${source.meta.basename}:${index}`} className="rounded border border-border bg-background px-2 py-1.5">
                    <summary className="cursor-pointer text-xs font-medium">
                      {source.meta.basename}
                      {index === evidence.evidence.primary_index ? ' — most useful' : ''}
                      {source.meta.truncated ? ' — trimmed to fit' : ''}
                      {source.meta.stale ? ' — older file' : ''}
                    </summary>
                    <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded bg-muted p-2 text-[11px] leading-relaxed">
                      {source.text}
                    </pre>
                  </details>
                ))}
              </div>
            )}
          </div>

          {evidence?.triage.matched && evidence.triage.solution_markdown && (
            <div className="rounded-lg border border-primary/30 bg-primary/10 p-4">
              <p className="mb-2 text-xs font-semibold uppercase tracking-wider text-primary">Fix that often helps</p>
              <div className="prose prose-sm max-w-none text-foreground dark:prose-invert">
                <ReactMarkdown
                  allowedElements={['p', 'strong', 'em', 'code', 'pre', 'ul', 'ol', 'li', 'blockquote', 'br']}
                  unwrapDisallowed
                >
                  {evidence.triage.solution_markdown}
                </ReactMarkdown>
              </div>
            </div>
          )}

          {recoverySnapshotId && (
            <div className="flex items-center justify-between gap-3 rounded-lg border border-border bg-muted p-3 text-xs text-muted-foreground">
              <span>We saved a backup before changing anything. You can undo the test.</span>
              <button
                onClick={() => {
                  void restoreInvestigationSnapshot()
                    .then(() => discardRecoverySnapshot())
                    .then(onClose)
                    .catch((cause) => {
                      const msg = formatError(cause);
                      const l = msg.toLowerCase();
                      if (l.includes('not found') || l.includes('not_found') || l.includes('unavailable') || l.includes('no backup')) {
                        clearRecoverySnapshotState();
                        onClose();
                      } else {
                        setError(`We couldn't put your mods back: ${msg}.`);
                      }
                    });
                }}
                disabled={loading}
                className="shrink-0 rounded-md border border-border px-2 py-1 font-medium hover:bg-accent disabled:opacity-50"
              >
                Undo and close
              </button>
            </div>
          )}
          {/* AI Assistant panel or suspect list — gated by ai_chat_enabled */}
          {aiChatEnabled && showAiAssistant ? (
            <div className="h-[480px] space-y-2">
              <button
                onClick={() => setShowAiAssistant(false)}
                className="text-xs text-primary hover:underline"
              >
                Back to suspects
              </button>
              <AiAssistant
                instanceId={instanceId}
                crashLog={crashLogText || manualLogText || null}
                crashSignatures={JSON.stringify(result.signature_name ?? null)}
                suspects={JSON.stringify(result.suspects)}
                onClose={() => setShowAiAssistant(false)}
              />
            </div>
          ) : (
            <>
              {/* Suspect list */}
              {suspects.length > 0 && (
                <div className="space-y-3">
                  <p className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                    Mods that might be causing this
                  </p>
                  {suspects.map((suspect, idx) => (
                    <SuspectCard
                      key={suspect.filename}
                      suspect={suspect}
                      rank={idx + 1}
                      isTop={idx === 0}
                      action={idx === 0 ? actionCard : undefined}
                      onAction={idx === 0 ? handleDisableAndRelaunch : undefined}
                      loading={loading}
                    />
                  ))}
                </div>
              )}

              {/* Ruled out */}
              <RuledOutList ruledOut={ruled_out} />

              {/* AI Explain toggle — gated by ai_chat_enabled */}
              {aiChatEnabled && !aiExplanation && !aiLoading && !aiError && (
                <button
                  onClick={handleAiExplain}
                  className="w-full rounded-lg border border-border bg-card px-3 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
                >
                  Explain this crash in plain language
                </button>
              )}

              {aiChatEnabled && aiLoading && (
                <div className="flex items-center gap-2 rounded-xl border border-border bg-card p-4 text-sm text-muted-foreground">
                  <div role="status" aria-label="Getting plain-language explanation" className="h-4 w-4 animate-spin rounded-full border-2 border-primary border-t-transparent" />
                  Getting a plain-language explanation…
                </div>
              )}

              {aiChatEnabled && aiError === 'connect-github' && (
                <div className="rounded-xl border border-primary/20 bg-primary/5 p-4 text-sm text-muted-foreground">
                  The AI helper isn’t connected.{' '}
                  <span className="text-primary">Connect GitHub in Settings</span> to get a plain-language explanation.
                </div>
              )}

              {aiChatEnabled && aiError && aiError !== 'connect-github' && (
                <div className="rounded-xl border border-destructive/30 bg-destructive/10 p-4 text-sm text-destructive">
                  {aiError}
                </div>
              )}

              {aiChatEnabled && aiExplanation && (
                <div className="rounded-xl border border-primary/20 bg-primary/5 p-4">
                  <div className="flex items-center justify-between mb-2">
                    <p className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                      Here’s what might have happened
                    </p>
                    <button
                      onClick={() => setAiExplanation(null)}
                      className="text-xs text-muted-foreground hover:text-foreground transition-colors"
                    >
                      Dismiss
                    </button>
                  </div>
                  <p className="text-sm whitespace-pre-wrap">{aiExplanation}</p>
                </div>
              )}

              {/* Post-launch confirmation */}
              {pendingTest && (
                <div className="rounded-xl border border-primary/30 bg-primary/10 p-4">
                  <div className="flex items-center gap-2 mb-1">
                    <div className="h-3 w-3 animate-pulse rounded-full bg-primary" />
                    <p className="text-sm font-medium">Game is running without “{pendingTest.filename}”</p>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    Play for a bit, then close the game. When you’re back here, we’ll check if it helped.
                  </p>
                </div>
              )}

              {experimentNotice && (
                <div className="rounded-xl border border-border bg-muted p-4 text-sm text-muted-foreground">
                  {experimentNotice}
                </div>
              )}

              {postLaunch && (
                <FixConfirmation
                  filename={postLaunch.filename}
                  onFix={handleFixConfirmed}
                  onStillCrashing={handleStillCrashing}
                  loading={loading}
                />
              )}

              {/* Triage banner */}
              {suggested_action.kind === 'ShowTriageBanner' && (
                <TriageBanner
                  modId={suggested_action.mod_id}
                  onViewTriage={handleViewTriage}
                />
              )}

              {/* No suspects */}
              {suggested_action.kind === 'NoSuspects' && (
                <div className="rounded-xl border border-border bg-card p-4">
                  <p className="text-sm text-muted-foreground">
                    We couldn’t tell which mod caused this. It might not be a mod issue at all. Try choosing a different crash file or pasting your crash text directly.
                  </p>
                </div>
              )}

              {/* Guided bisect — the systematic fallback when scoring suspects
                  has not produced an obvious answer, and the thing users
                  otherwise do by hand across a dozen launches. */}
              <ModBisectPanel
                instanceId={instanceId}
                primeSuspects={suspects.map((suspect) => suspect.filename)}
                // The same launch path the single-suspect test uses, so a
                // trial gets the health checks and launch mode of a real run.
                onLaunch={() => onLaunch(() => { stackedHealthDialogRef.current = true; })}
              />

              {/* Success */}
              {success && <SuccessBanner message={success} />}
            </>
          )}
        </div>
        </DialogContent>
      </Dialog>

      {/* Disable dependency prompt */}
      {disablePlanTarget && (
        <DependencyPrompt
          title={`“${disablePlanTarget.originalFilename}” needs other mods`}
          actionLabel="Turn off selected and try again"
          candidates={disablePlanTarget.plan.dependents.map((d) => ({
            key: d.mod_id,
            label: d.mod_id,
            requirement: d.requirement,
            source: d.source,
          }))}
          onConfirm={handleDisableConfirm}
          onCancel={() => setDisablePlanTarget(null)}
        />
      )}
    </>
  );
}
