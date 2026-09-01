import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import {
  type InstallIntent,
  type ResolvedArtifact,
  type ResolvedInstallPlan,
  type InstallOutcome,
  type ProgressEvent,
  type DepConflict,
  type ResolvedDep,
  resolveInstallPlan,
  applyInstallPlan,
  cancelInstall,
  subscribeProgress,
  planNeedsUserReview,
} from '../lib/installFlow';
import { formatError, getSetting, parseLauncherError, restoreSnapshot } from '../lib/tauri';
import { emitTourSignal } from '../features/tour/tourSignals';
import { LoaderChooser } from './LoaderChooser';

// ---------------------------------------------------------------------------
// User choices model
// ---------------------------------------------------------------------------

interface PlanChoices {
  optionalIncluded: Set<string>;
  conflictResolutions: Map<string, string>;
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

type FlowState =
  | { phase: 'resolving'; plan?: ResolvedInstallPlan; error?: string }
  | { phase: 'review'; plan: ResolvedInstallPlan; choices: PlanChoices; dirty: boolean }
  | { phase: 'executing'; plan: ResolvedInstallPlan; progress: ProgressEvent }
  | { phase: 'result'; outcome: InstallOutcome }
  | { phase: 'error'; message: string; retryable: boolean; code?: string }
  | { phase: 'closed' };

type FlowAction =
  | { type: 'resolved'; plan: ResolvedInstallPlan }
  | { type: 'resolve-error'; error: string; code?: string }
  | { type: 'patch-choice'; modJarId: string; included: boolean }
  | { type: 'resolve-conflict'; conflictId: string; resolution: string }
  | { type: 'confirm' }
  | { type: 'confirm-replan' }
  | { type: 'progress'; event: ProgressEvent }
  | { type: 'outcome'; outcome: InstallOutcome }
  | { type: 'fail'; message: string; retryable: boolean }
  | { type: 'retry' }
  | { type: 'close' };

function flowReducer(state: FlowState, action: FlowAction): FlowState {
  switch (action.type) {
    case 'resolved':
      if (state.phase !== 'resolving') return state;
      return {
        phase: 'review',
        plan: action.plan,
        choices: defaultChoices(action.plan),
        dirty: false,
      };

    case 'resolve-error':
      return { phase: 'error', message: action.error, retryable: true, code: action.code };

    case 'patch-choice':
      if (state.phase !== 'review') return state;
      return {
        ...state,
        choices: {
          ...state.choices,
          optionalIncluded: (() => {
            const next = new Set(state.choices.optionalIncluded);
            if (action.included) next.add(action.modJarId);
            else next.delete(action.modJarId);
            return next;
          })(),
        },
        dirty: true,
      };

    case 'resolve-conflict':
      if (state.phase !== 'review') return state;
      return {
        ...state,
        choices: {
          ...state.choices,
          conflictResolutions: new Map(state.choices.conflictResolutions).set(action.conflictId, action.resolution),
        },
        dirty: true,
      };

    case 'confirm':
      if (state.phase !== 'review') return state;
      return {
        phase: 'executing',
        plan: state.plan,
        progress: {
          planId: state.plan.fingerprint,
          phase: 'staging' as const,
          step: 0, totalSteps: 0, bytesDownloaded: 0, bytesTotal: 0,
          message: 'Starting…',
        },
      };

    case 'progress':
      if (state.phase !== 'executing') return state;
      return { ...state, progress: action.event };

    case 'outcome':
      return { phase: 'result', outcome: action.outcome };

    case 'fail':
      return { phase: 'error', message: action.message, retryable: action.retryable };

    case 'retry':
      return { phase: 'resolving', plan: state.phase === 'review' ? state.plan : undefined };

    case 'close':
      return { phase: 'closed' };

    default:
      return state;
  }
}

function defaultChoices(plan: ResolvedInstallPlan): PlanChoices {
  // For bulk installs optional dependencies default to UNselected: most are
  // author recommendations rather than functional requirements, and selecting
  // a dozen extras by accident is worse than opting in deliberately. Single
  // installs keep the previous include-by-default behavior.
  const includeOptionalByDefault = plan.intent.action.type !== 'batch-install';
  return {
    optionalIncluded: new Set(
      includeOptionalByDefault
        ? plan.dependencies
            .filter((d) => d.requirement === 'optional')
            .map((d) => d.modJarId)
        : [],
    ),
    conflictResolutions: new Map(
      plan.conflicts
        .filter((c) => c.chosen)
        .map((c) => [c.conflictId, c.chosen!]),
    ),
  };
}

function failedBatchItemId(intent: InstallIntent, message: string): string | undefined {
  if (intent.action.type !== 'batch-install') return undefined;
  const match = message.match(/(?:curated|Modrinth|manual) item '([^']+)'/);
  const itemId = match?.[1];
  return itemId && intent.action.items.some((item) => item.itemId === itemId)
    ? itemId
    : undefined;
}

function failedBlockingBatchItemId(plan: ResolvedInstallPlan): string | undefined {
  if (plan.intent.action.type !== 'batch-install') return undefined;
  const error = plan.blockingErrors.find((candidate) =>
    candidate.code === 'ERR_HASH_UNAVAILABLE'
    || candidate.message.includes('has no acceptable published hash'),
  );
  const itemId = error?.message.match(/^([^ ]+) has no acceptable published hash/)?.[1];
  return itemId && plan.intent.action.items.some((item) => item.itemId === itemId)
    ? itemId
    : undefined;
}

/**
 * Batch items to skip when the user declines a loader change during a batch
 * install. Prefer the batch items whose ids match the declaring mod ids of the
 * unsatisfied loader requirements; if no item can be matched, skip the whole
 * batch (the safest interpretation of "skip the incompatible mods").
 */
function loaderMismatchSkipItems(plan: ResolvedInstallPlan): string[] {
  if (plan.intent.action.type !== 'batch-install') return [];
  const batchIds = plan.intent.action.items.map((item) => item.itemId);
  if (batchIds.length === 0) return [];
  const choice = plan.pendingChoices.find((candidate) => candidate.type === 'loader-change');
  if (!choice) return batchIds;
  const unsatisfied = new Set<string>();
  for (const requirement of choice.requirements) {
    if (requirement.verdict === 'satisfied') continue;
    const ids = requirement.declaring_mod_ids
      ?? (requirement.declaring_mod_id ? [requirement.declaring_mod_id] : []);
    for (const id of ids) unsatisfied.add(id.toLowerCase());
  }
  if (unsatisfied.size === 0) return batchIds;
  const matched = batchIds.filter((id) => unsatisfied.has(id.toLowerCase()));
  return matched.length > 0 ? matched : batchIds;
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface InstallFlowProps {
  intent: InstallIntent;
  instanceName: string;
  onOpenInstance?: (instanceId: string) => void;
  onClose?: () => void;
  onSuccess?: (instanceId: string) => void;
  onBackgroundStart?: (plan: ResolvedInstallPlan) => void;
  background?: boolean;
  open: boolean;
  /** Pre-resolved plan (e.g. resolved by the caller in the background). Used once on open. */
  initialPlan?: ResolvedInstallPlan | null;
  /** Apply failure promoted from a background task. */
  initialError?: { message: string; code?: string; skipItemId?: string } | null;
  /** When set, a clean plan starts immediately in the background. */
  autoBackground?: boolean;
  /** When enabled with autoBackground, dependency details are also skipped. */
  alwaysAutoConfirm?: boolean;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function InstallFlow({
  intent,
  instanceName,
  onOpenInstance,
  onClose,
  onSuccess,
  onBackgroundStart,
  background = false,
  open,
  initialPlan,
  initialError,
  autoBackground = false,
  alwaysAutoConfirm = false,
}: InstallFlowProps) {
  const [state, dispatch] = useReducer(flowReducer, { phase: 'closed' } as FlowState);
  const [resolutionIntent, setResolutionIntent] = useState(intent);
  const [settingAutoConfirmClean, setSettingAutoConfirmClean] = useState(false);
  const [settingAlwaysAutoConfirm, setSettingAlwaysAutoConfirm] = useState(false);
  const initialPlanRef = useRef<ResolvedInstallPlan | null>(initialPlan ?? null);
  const recheckPlanRef = useRef<ResolvedInstallPlan | null>(null);
  const forceFinalReviewRef = useRef(false);
  const [newlyAddedFiles, setNewlyAddedFiles] = useState<Set<string>>(new Set());
  const [reviewNotice, setReviewNotice] = useState<string | null>(null);

  useEffect(() => {
    if (!background) return;
    let cancelled = false;
    void Promise.all([
      getSetting('install_auto_confirm_clean'),
      getSetting('install_always_auto_confirm'),
    ])
      .then(([value, alwaysValue]) => {
        if (!cancelled) {
          setSettingAutoConfirmClean(value === true || value === 'true' || value === 1 || value === '1');
          setSettingAlwaysAutoConfirm(alwaysValue === true || alwaysValue === 'true' || alwaysValue === 1 || alwaysValue === '1');
        }
      })
      .catch(() => {
        // Fail closed: a setting read failure must not auto-apply changes.
      });
    return () => { cancelled = true; };
  }, [background]);

  // Start resolving on first open.
  useEffect(() => {
    if (!open) return;
    setResolutionIntent(intent);
    recheckPlanRef.current = null;
    forceFinalReviewRef.current = false;
    setNewlyAddedFiles(new Set());
    setReviewNotice(null);
    if (initialError) {
      initialPlanRef.current = null;
      dispatch({ type: 'resolve-error', error: initialError.message, code: initialError.code });
      return;
    }
    dispatch({ type: 'retry' });
  }, [open, intent, initialError]);

  // Resolve when entering resolving phase. A pre-resolved plan skips the
  // network round-trip entirely.
  useEffect(() => {
    if (state.phase !== 'resolving') return;
    const preResolved = initialPlanRef.current;
    initialPlanRef.current = null;
    if (preResolved) {
      dispatch({ type: 'resolved', plan: preResolved });
      return;
    }
    let cancelled = false;
    (async () => {
        try {
          const plan = await resolveInstallPlan(resolutionIntent);
          if (!cancelled) {
            const previousPlan = recheckPlanRef.current;
            if (previousPlan) {
              recheckPlanRef.current = null;
              forceFinalReviewRef.current = true;
              const previousFiles = new Set(previousPlan.filesToAdd.map((file) => file.targetFilename));
              setNewlyAddedFiles(new Set(
                plan.filesToAdd
                  .map((file) => file.targetFilename)
                  .filter((filename) => !previousFiles.has(filename)),
              ));
              setReviewNotice('New options have been checked. Review the highlighted additions before installing.');
            }
            dispatch({ type: 'resolved', plan });
          }
        } catch (e) {
          if (!cancelled) {
            recheckPlanRef.current = null;
            forceFinalReviewRef.current = false;
            const parsed = parseLauncherError(e);
          dispatch({ type: 'resolve-error', error: parsed.message, code: parsed.code });
        }
      }
    })();
    return () => { cancelled = true; };
  }, [state.phase, resolutionIntent]);

  // When the resolved plan needs no choices and nothing blocks it, proceed in
  // the background instead of waiting on the focused review dialog.
  useEffect(() => {
    if (!background || (!autoBackground && !settingAutoConfirmClean)) return;
    if (recheckPlanRef.current || forceFinalReviewRef.current) return;
    if (state.phase !== 'review' || state.dirty) return;
    const effectiveAutoConfirm = autoBackground || settingAutoConfirmClean;
    if (planNeedsUserReview(state.plan, {
      ignoreDependencies: (alwaysAutoConfirm || settingAlwaysAutoConfirm)
        && effectiveAutoConfirm,
    })) return;
    onBackgroundStart?.(state.plan);
    dispatch({ type: 'close' });
    onClose?.();
  }, [alwaysAutoConfirm, autoBackground, background, settingAlwaysAutoConfirm, settingAutoConfirmClean, state, onBackgroundStart, onClose]);

  // Execute plan.
  useEffect(() => {
    if (state.phase !== 'executing') return;
    let cancelled = false;
    let unsubscribe: (() => void) | undefined;
    (async () => {
      try {
        unsubscribe = await subscribeProgress(state.plan.fingerprint, (event) => {
          dispatch({ type: 'progress', event });
        });
        const outcome = await applyInstallPlan(state.plan);
        if (!cancelled) {
          dispatch({ type: 'outcome', outcome });
          // The guided walkthrough waits on the install actually landing, which
          // the dialog closing cannot distinguish from a cancel.
          if (outcome.type === 'success') emitTourSignal('install-completed');
        }
      } catch (e) {
        if (!cancelled) dispatch({ type: 'fail', message: formatError(e), retryable: false });
      }
    })();
    return () => {
      cancelled = true;
      unsubscribe?.();
    };
  }, [state.phase]);

  const handleCancel = useCallback(() => {
    if (state.phase === 'executing') {
      void cancelInstall(state.plan.fingerprint).catch(() => {
        // The executor may already be past its cancellable staging phase.
      });
      return; // Dialog stays open — user must wait for outcome.
    }
    if (state.phase === 'review') {
      void cancelInstall(state.plan.fingerprint).catch(() => {});
      dispatch({ type: 'close' });
      onClose?.();
      return;
    }
    if (state.phase === 'resolving' || state.phase === 'error') {
      dispatch({ type: 'close' });
      onClose?.();
    }
  }, [state, onClose]);

  const handleConfirm = useCallback(() => {
    if (state.phase !== 'review') return;
    if (state.dirty || state.plan.pendingChoices.length > 0) {
      recheckPlanRef.current = state.plan;
      forceFinalReviewRef.current = false;
      setNewlyAddedFiles(new Set());
      setReviewNotice(null);
      setResolutionIntent({
        ...resolutionIntent,
        optionalDeps: {
          type: 'include',
          deps: [...state.choices.optionalIncluded].sort(),
        },
        overrides: {
          ...resolutionIntent.overrides,
          forceConflictResolution: Object.fromEntries(state.choices.conflictResolutions),
        },
      });
      dispatch({ type: 'retry' });
      return;
    }
    if (background) {
      forceFinalReviewRef.current = false;
      onBackgroundStart?.(state.plan);
      dispatch({ type: 'close' });
      onClose?.();
    } else {
      dispatch({ type: 'confirm' });
    }
  }, [background, onBackgroundStart, onClose, state, resolutionIntent]);

  const handleClose = useCallback(() => {
    dispatch({ type: 'close' });
    onClose?.();
  }, [onClose]);

  // Invalidate the update cache on success so the badge does not linger. The
  // cache is an invalidated view (install path stays unaware); clearing is
  // honest and cheap, re-check is eager network work the sweep will do later.
  //
  // Fired exactly once per success via a ref, and reset when we leave the
  // success state so a later install in the same mount still signals. Callers
  // pass an inline closure, so a plain dependency on `onSuccess` would re-fire
  // on every render — and if the handler sets state, that is an infinite loop.
  const signalledSuccessRef = useRef(false);
  const installSucceeded = state.phase === 'result' && state.outcome.type === 'success';
  useEffect(() => {
    if (!installSucceeded) {
      signalledSuccessRef.current = false;
      return;
    }
    if (signalledSuccessRef.current) return;
    signalledSuccessRef.current = true;
    onSuccess?.(intent.targetInstance);
  }, [installSucceeded, intent.targetInstance, onSuccess]);

  /**
   * Approve switching the instance loader to a signed-catalog compatible
   * version. The backend folds the switch into the same atomic install
   * transaction, so we just re-resolve with the approval recorded.
   */
  const handleChooseLoaderVersion = useCallback((version: string) => {
    setResolutionIntent((current) => ({
      ...current,
      overrides: {
        ...current.overrides,
        approveLoaderVersion: version,
      },
    }));
    dispatch({ type: 'retry' });
  }, []);

  /**
   * Escape hatch from a loader mismatch: don't switch the loader, skip the
   * mods that don't fit it. A single install is skipped outright; a batch
   * re-resolves with the incompatible items excluded.
   */
  const handleSkipLoaderMismatch = useCallback(() => {
    const action = resolutionIntent.action;
    if (action.type === 'install') {
      // Nothing else to install for a single mod — closing means "don't install".
      dispatch({ type: 'close' });
      onClose?.();
      return;
    }
    // The mismatch escape hatch is only offered on the review screen, where a
    // resolved plan is guaranteed to exist.
    if (action.type === 'batch-install' && state.phase === 'review') {
      const itemsToSkip = loaderMismatchSkipItems(state.plan);
      setResolutionIntent((current) => ({
        ...current,
        overrides: {
          ...current.overrides,
          skipItems: [
            ...new Set([...(current.overrides.skipItems ?? []), ...itemsToSkip]),
          ],
        },
      }));
      dispatch({ type: 'retry' });
    }
  }, [resolutionIntent, state.phase, onClose]);

  const renderContent = () => {
    switch (state.phase) {
      case 'resolving':
        return <ResolvingView rechecking={Boolean(recheckPlanRef.current)} />;
      case 'review':
        {
          const skippedItemId = failedBlockingBatchItemId(state.plan);
          const onSkip = skippedItemId
            ? () => {
              setResolutionIntent((current) => ({
                ...current,
                overrides: {
                  ...current.overrides,
                  skipItems: [...new Set([...(current.overrides.skipItems ?? []), skippedItemId])],
                },
              }));
              dispatch({ type: 'retry' });
            }
            : undefined;
          return <ReviewView
           plan={state.plan}
           choices={state.choices}
           newlyAddedFiles={newlyAddedFiles}
           reviewNotice={reviewNotice}
           onToggleOptional={(id, inc) => dispatch({ type: 'patch-choice', modJarId: id, included: inc })}
           onResolveConflict={(id, res) => dispatch({ type: 'resolve-conflict', conflictId: id, resolution: res })}
             onRetry={() => dispatch({ type: 'retry' })}
             onSkip={onSkip}
             onChooseLoaderVersion={handleChooseLoaderVersion}
             onSkipLoaderMismatch={handleSkipLoaderMismatch}
            onConfirm={handleConfirm}
            onCancel={handleCancel}
         />;
        }
      case 'executing':
        return <ProgressView progress={state.progress} onCancel={handleCancel} />;
      case 'result':
        return <ResultView
          outcome={state.outcome}
          instanceId={intent.targetInstance}
          onOpenInstance={() => onOpenInstance?.(intent.targetInstance)}
          onClose={handleClose}
        />;
      case 'error':
        {
          const skippedItemId = initialError?.skipItemId ?? failedBatchItemId(resolutionIntent, state.message);
        return <ErrorView
          message={state.message}
          retryable={state.retryable}
          onRetry={() => dispatch({ type: 'retry' })}
          canTryClosest={state.code === 'ERR_VERSION_NOT_FOUND'}
          canSkip={Boolean(skippedItemId)}
          onSkip={() => {
            if (!skippedItemId) return;
            setResolutionIntent((current) => ({
              ...current,
              overrides: {
                ...current.overrides,
                skipItems: [...new Set([...(current.overrides.skipItems ?? []), skippedItemId])],
              },
            }));
            dispatch({ type: 'retry' });
          }}
          onTryClosest={() => {
            setResolutionIntent((current) => ({
              ...current,
              overrides: {
                ...current.overrides,
                allowClosestVersion: true,
              },
            }));
            dispatch({ type: 'retry' });
          }}
          onClose={handleClose}
        />;
        }
      default:
        return null;
    }
  };

  if (!open) return null;

  // Non-blocking corner panel — lets the user keep browsing, running health
  // checks, or opening other instances while resolving, reviewing, or
  // installing. The card expands in place instead of covering the app.
  // z-50 keeps it above the pack-indicator stacking context when both are
  // visible, so the review remains actionable; pack progress remains
  // readable alongside via vertical stacking.
  // It is still a dialog — a task the user is asked to act on — so it keeps
  // the role and an accessible name; aria-modal="false" is what states that
  // the rest of the app stays live behind it.
  return (
    <aside
      className="fixed bottom-4 right-4 z-[61] flex max-h-[85vh] w-[min(36rem,calc(100vw-2rem))] flex-col overflow-hidden rounded-xl border border-border bg-card shadow-2xl"
      data-tour="install-review-dialog"
      role="dialog"
      aria-modal="false"
      aria-labelledby="install-review-title"
      aria-live="polite"
    >
      <div className="flex shrink-0 items-start justify-between gap-3 border-b border-border bg-card px-4 py-3">
        <div className="min-w-0">
          <h2 id="install-review-title" className="truncate text-sm font-semibold">Review Instance Changes</h2>
          <p className="truncate text-xs text-muted-foreground">{instanceName}</p>
        </div>
        <button
          onClick={handleCancel}
          className="shrink-0 rounded-lg border border-input px-2.5 py-1 text-xs font-medium hover:bg-accent"
          aria-label="Close install panel"
        >
          Close
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        {renderContent()}
      </div>
    </aside>
  );
}

// ---------------------------------------------------------------------------
// Sub-views
// ---------------------------------------------------------------------------

function ResolvingView({ rechecking }: { rechecking: boolean }) {
  return (
    <div className="flex flex-col items-center justify-center py-8 gap-3">
      <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
      <p className="text-sm text-muted-foreground">
        {rechecking ? 'Checking newly selected dependencies…' : 'Resolving dependencies…'}
      </p>
    </div>
  );
}

function ReviewView({
  plan,
  choices,
  onToggleOptional,
  onResolveConflict,
  onRetry,
  onSkip,
  onChooseLoaderVersion,
  onSkipLoaderMismatch,
  onConfirm,
  onCancel,
  newlyAddedFiles,
  reviewNotice,
}: {
  plan: ResolvedInstallPlan;
  choices: PlanChoices;
  onToggleOptional: (id: string, inc: boolean) => void;
  onResolveConflict: (id: string, res: string) => void;
  onRetry: () => void;
  onSkip?: () => void;
  onChooseLoaderVersion: (version: string) => void;
  onSkipLoaderMismatch: () => void;
  onConfirm: () => void;
  onCancel: () => void;
  newlyAddedFiles: Set<string>;
  reviewNotice: string | null;
}) {
  const canInstall = plan.blockingErrors.length === 0;
  const hasUnresolvedBlockingConflict = plan.conflicts.some(
    (conflict) => conflict.blocking
      && !conflict.chosen
      && !choices.conflictResolutions.has(conflict.conflictId),
  );
  const needsReplan = plan.pendingChoices.length > 0;
  const loaderChoice = plan.pendingChoices.find((choice) => choice.type === 'loader-change');
  const needsLoaderDecision = Boolean(loaderChoice);
  const actionLabel = plan.intent.action.type === 'remove'
    ? 'Remove Safely'
    : plan.intent.action.type === 'batch-remove'
      ? 'Remove Selected Safely'
    : plan.intent.action.type === 'batch-update'
      ? 'Apply Updates'
      : plan.intent.action.type === 'batch-install'
        ? 'Install Batch'
        : 'Install';
  const selectedVersions = operationArtifacts(plan.operation)
    .map((artifact) => ({
      filename: artifact.filename,
      version: artifact.metadata.version,
      isNew: newlyAddedFiles.has(artifact.filename),
    }))
    .filter((artifact) => artifact.version);

  return (
    <div className="space-y-4">
      {reviewNotice && (
        <div className="rounded-lg border border-primary/30 bg-primary/10 p-3 text-xs text-primary">
          {reviewNotice}
        </div>
      )}

      {/* Loader version change — show the launcher chooser instead of blocking */}
      {loaderChoice && (
        <LoaderChooser
          loader={loaderChoice.loader}
          currentVersion={loaderChoice.currentVersion}
          recommendedVersion={loaderChoice.recommendedVersion}
          compatibleVersions={loaderChoice.compatibleVersions}
          requirements={loaderChoice.requirements.map((requirement) => ({
            targetId: requirement.target_id,
            versionRanges: requirement.version_ranges,
            candidateVersion: requirement.candidate_version,
            verdict: requirement.verdict,
            modIds: requirement.declaring_mod_ids
              ?? (requirement.declaring_mod_id ? [requirement.declaring_mod_id] : []),
          }))}
          conflicts={loaderChoice.conflicts}
          onChoose={onChooseLoaderVersion}
          onSkip={onSkipLoaderMismatch}
          skipLabel={plan.intent.action.type === 'batch-install'
            ? 'Skip incompatible mods instead'
            : 'Skip this mod instead'}
        />
      )}

      {/* Approved loader switch, committed atomically with the file changes */}
      {plan.loaderChange && (
        <div className="rounded-lg border border-green-600/30 bg-green-500/10 p-3 text-xs text-green-700 dark:text-green-300">
          This change will switch the {plan.loaderChange.loader} loader from{' '}
          {plan.loaderChange.fromVersion} to {plan.loaderChange.toVersion} in the same
          atomic transaction as the mod files.
        </div>
      )}

      {/* Warnings */}
      {plan.warnings.length > 0 && (
        <div className="rounded-lg bg-amber-50 dark:bg-amber-900/20 p-3 text-xs text-amber-700 dark:text-amber-300 space-y-1">
          {plan.warnings.map((w, i) => <p key={i}>{w.message}</p>)}
        </div>
      )}

      {/* Blocking errors */}
      {plan.blockingErrors.length > 0 && (
        <div className="rounded-lg bg-destructive/10 p-3 text-xs text-destructive space-y-1">
          {plan.blockingErrors.map((e, i) => <p key={i}>{e.message}</p>)}
        </div>
      )}

      {/* Dependencies */}
      {plan.dependencies.length > 0 && (
        <div>
          <h4 className="text-sm font-semibold mb-2">Dependencies</h4>
          <div className="space-y-1 max-h-40 overflow-y-auto">
            {plan.dependencies.map((dep, i) => (
              <DepRow
                key={i}
                dep={dep}
                checked={choices.optionalIncluded.has(dep.modJarId)}
                onToggle={onToggleOptional}
                highlighted={dependencyIsNew(dep, newlyAddedFiles)}
              />
            ))}
          </div>
        </div>
      )}

      {/* Conflicts */}
      {plan.conflicts.length > 0 && (
        <div>
          <h4 className="text-sm font-semibold mb-2">Conflicts</h4>
          <div className="space-y-2">
            {plan.conflicts.map((c, i) => (
              <ConflictRow key={i} conflict={c} selected={choices.conflictResolutions.get(c.conflictId)} onSelect={(r) => onResolveConflict(c.conflictId, r)} />
            ))}
          </div>
        </div>
      )}

      {/* File changes */}
      {(plan.filesToAdd.length > 0 || plan.filesToRemove.length > 0) && (
        <div>
          <h4 className="text-sm font-semibold mb-2">File Changes</h4>
          <p className="text-xs text-muted-foreground">
            {plan.filesToAdd.length > 0 && <span>+{plan.filesToAdd.length} to add </span>}
            {plan.filesToRemove.length > 0 && <span>-{plan.filesToRemove.length} to remove </span>}
            {plan.filesToDisable.length > 0 && <span>~{plan.filesToDisable.length} to disable</span>}
          </p>
        </div>
      )}

      {selectedVersions.length > 0 && (
        <div>
          <h4 className="text-sm font-semibold mb-2">Versions to Install</h4>
          <div className="space-y-1 text-xs text-muted-foreground">
            {selectedVersions.map((artifact, index) => (
              <p key={`${artifact.filename}-${index}`}>
                <span className={artifact.isNew ? 'rounded bg-primary/15 px-1 font-medium text-primary' : 'font-medium text-foreground'}>
                  {artifact.isNew ? 'NEW ' : ''}{artifact.version}
                </span>{' '}
                {artifact.filename}
              </p>
            ))}
          </div>
        </div>
      )}

      {newlyAddedFiles.size > 0 && (
        <div className="rounded-lg border border-primary/30 bg-primary/10 p-3">
          <h4 className="text-sm font-semibold text-primary mb-2">New Items From Optional Dependencies</h4>
          <ul className="space-y-1 text-xs text-primary">
            {[...newlyAddedFiles].map((filename) => <li key={filename}>+ {filename}</li>)}
          </ul>
        </div>
      )}

      {/* Snapshot info */}
      <div className="text-xs text-muted-foreground">
        Snapshot: {plan.snapshot.label} ({formatBytes(plan.snapshot.estimatedBytes)})
      </div>

      {/* Actions */}
      <div className="flex justify-end gap-2 pt-2">
        {plan.blockingErrors.length > 0 && (
          <button onClick={onRetry} className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent">
            Retry Resolution
          </button>
        )}
        {onSkip && (
          <button onClick={onSkip} className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent">
            Skip This Mod
          </button>
        )}
        <button onClick={onCancel} className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent">Cancel</button>
        <button
          onClick={onConfirm}
          disabled={!canInstall || hasUnresolvedBlockingConflict || needsLoaderDecision}
          data-tour="install-review-confirm"
          className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
        >
          {needsLoaderDecision
            ? 'Choose a loader version or skip'
            : hasUnresolvedBlockingConflict
              ? 'Resolve Conflicts First'
              : !canInstall
                ? 'Cannot Apply'
                : needsReplan
                  ? 'Review Selected Changes'
                  : actionLabel}
        </button>
      </div>
    </div>
  );
}

function DepRow({ dep, checked, onToggle, highlighted }: { dep: ResolvedDep; checked: boolean; onToggle: (id: string, inc: boolean) => void; highlighted: boolean }) {
  const isOptional = dep.requirement === 'optional';
  const displayName = dep.displayName ?? dep.modJarId;
  return (
    <div className={`flex items-center gap-2 rounded px-1 text-sm ${highlighted ? 'bg-primary/10 ring-1 ring-primary/30' : ''}`}>
      {isOptional && (
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => onToggle(dep.modJarId, e.target.checked)}
          className="rounded"
          aria-label={`Include optional dependency ${displayName}`}
        />
      )}
      <span className={`min-w-0 truncate ${isOptional ? '' : 'font-medium'}`} title={displayName}>
        {displayName}
      </span>
      <span className="shrink-0 text-xs text-muted-foreground">{dep.requirement}</span>
      {dep.disposition.type === 'reuse-existing' && (
        <span className="shrink-0 text-xs text-green-600">✓ already installed</span>
      )}
      {dep.disposition.type === 'install-candidate' && (
        <span className="shrink-0 text-xs text-muted-foreground">⬇ will be installed</span>
      )}
      {dep.disposition.type === 'included-in-batch' && (
        <span className="shrink-0 text-xs text-green-600">✓ included in this batch</span>
      )}
      {dep.disposition.type === 'unresolved' && (
        <span className="shrink-0 text-xs text-destructive" title={dep.disposition.reason}>⚠ unresolved</span>
      )}
      {dep.pageUrl?.startsWith('https://') && (
        <a
          href={dep.pageUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="shrink-0 text-xs text-primary hover:underline"
        >
          View mod page ↗
        </a>
      )}
      {highlighted && <span className="shrink-0 text-xs font-medium text-primary">new</span>}
    </div>
  );
}

function dependencyIsNew(dep: ResolvedDep, newlyAddedFiles: Set<string>): boolean {
  return dep.disposition.type === 'install-candidate'
    && newlyAddedFiles.has(dep.disposition.artifact.filename);
}

function operationArtifacts(operation: ResolvedInstallPlan['operation']): ResolvedArtifact[] {
  switch (operation.type) {
    case 'install': return [operation.artifact];
    case 'update': return [operation.newArtifact];
    case 'batch-install':
    case 'batch-update':
    case 'reconcile': return operation.operations.flatMap(operationArtifacts);
    default: return [];
  }
}

function ConflictRow({ conflict, selected, onSelect }: { conflict: DepConflict; selected?: string; onSelect: (r: string) => void }) {
  return (
    <div className="rounded border border-border bg-muted p-2 text-sm">
      <p className="text-xs">{conflict.message}</p>
      <div className="flex gap-2 mt-1">
        {conflict.resolutionOptions.map((opt) => (
          <button
            key={opt}
            onClick={() => onSelect(opt)}
            className={`rounded px-2 py-0.5 text-xs border ${selected === opt ? 'bg-primary text-primary-foreground border-primary' : 'border-border hover:bg-accent'}`}
          >
            {opt}
          </button>
        ))}
      </div>
    </div>
  );
}

function ProgressView({ progress, onCancel }: { progress: ProgressEvent; onCancel: () => void }) {
  const phaseLabel = installPhaseLabel(progress.phase);
  const label = progress.message || phaseLabel;
  const hasBytes = progress.bytesTotal > 0;
  const hasSteps = progress.totalSteps > 0;
  const pct = hasBytes
    ? Math.round((progress.bytesDownloaded / progress.bytesTotal) * 100)
    : hasSteps
      ? Math.round((progress.step / progress.totalSteps) * 100)
      : null;

  return (
    <div className="space-y-4 py-4">
      <div className="flex items-center gap-3">
        <div className="h-5 w-5 animate-spin rounded-full border-2 border-primary border-t-transparent" />
        <div className="min-w-0">
          <p className="text-sm font-medium">{phaseLabel}</p>
          <p className="truncate text-xs text-muted-foreground" title={label}>{label}</p>
        </div>
      </div>
      {pct !== null && (
        <div className="space-y-1">
          <div className="h-2 rounded-full bg-muted overflow-hidden">
            <div className="h-full bg-primary transition-all duration-300" style={{ width: `${pct}%` }} />
          </div>
          <div className="flex justify-between text-xs text-muted-foreground">
            <span>{pct}%</span>
            {hasBytes ? (
              <span>{formatBytes(progress.bytesDownloaded)} / {formatBytes(progress.bytesTotal)}</span>
            ) : (
              <span>File {Math.min(progress.step, progress.totalSteps)} of {progress.totalSteps}</span>
            )}
          </div>
        </div>
      )}
      {pct === null && (
        <div className="h-2 overflow-hidden rounded-full bg-muted">
          <div className="h-full w-1/3 animate-pulse rounded-full bg-primary" />
        </div>
      )}
      <div className="flex justify-end">
        <button onClick={onCancel} className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent">Cancel</button>
      </div>
    </div>
  );
}

function installPhaseLabel(phase: ProgressEvent['phase']): string {
  switch (phase) {
    case 'resolving': return 'Preparing installation';
    case 'staging': return 'Loading files';
    case 'snapshotting': return 'Creating recovery snapshot';
    case 'applying': return 'Applying instance changes';
    case 'health-scan': return 'Checking pack health';
    case 'done': return 'Finishing installation';
    case 'failed': return 'Installation failed';
    case 'cancelled': return 'Installation cancelled';
    default: return 'Installing';
  }
}

function ResultView({ outcome, instanceId, onOpenInstance, onClose }: {
  outcome: InstallOutcome;
  instanceId: string;
  onOpenInstance: () => void;
  onClose: () => void;
}) {
  const [rollbackState, setRollbackState] = useState<'idle' | 'restoring' | 'restored'>('idle');
  const [rollbackError, setRollbackError] = useState<string | null>(null);
  const snapshotId =
    outcome.type === 'success' || outcome.type === 'health-rollback' || outcome.type === 'failed'
      ? outcome.snapshotId
      : undefined;
  const canRestore =
    rollbackState !== 'restored'
    && (
      outcome.type === 'success'
      || outcome.type === 'health-rollback'
      || (outcome.type === 'failed' && Boolean(outcome.snapshotId) && !outcome.rollbackPerformed)
    );

  const rollback = async () => {
    if (!snapshotId) return;
    setRollbackState('restoring');
    setRollbackError(null);
    try {
      await restoreSnapshot(instanceId, snapshotId);
      setRollbackState('restored');
    } catch (cause) {
      setRollbackState('idle');
      setRollbackError(formatError(cause));
    }
  };

  return (
    <div className="space-y-4 py-4">
      {outcome.type === 'success' && (
        <>
          <div className="rounded-lg bg-green-500/10 p-3 text-sm text-green-700 dark:text-green-300">
            All verified changes were applied successfully.
          </div>
          <p className="text-xs text-muted-foreground">
            Recovery snapshot: {outcome.snapshotId}
          </p>
        </>
      )}
      {outcome.type === 'health-rollback' && (
        <div className="space-y-3">
          <div className="rounded-lg border border-amber-500 bg-amber-500/10 p-3 text-sm text-amber-700 dark:text-amber-300">
            <p className="font-semibold">Health check found blockers — install kept for repair</p>
            <p className="mt-1 text-xs">
              The new files are still installed so you can see what failed. The recovery snapshot <span className="font-mono">{outcome.snapshotId.slice(0, 8)}</span> is kept for a one-click rollback. Fix the issue or roll back when ready.
            </p>
          </div>
          {outcome.healthReport.blockers.length > 0 && (
            <div className="space-y-2">
              <h4 className="text-sm font-semibold">Blockers ({outcome.healthReport.blockers.length})</h4>
              <div className="max-h-48 space-y-2 overflow-y-auto pr-1">
                {outcome.healthReport.blockers.map((b, i) => (
                  <div key={i} className="rounded border border-destructive bg-destructive/10 p-2 text-sm">
                    <p className="text-destructive">{b.message}</p>
                    {b.suggested_action && <p className="mt-1 text-xs text-muted-foreground">{b.suggested_action}</p>}
                    {b.filename && <p className="mt-1 text-xs text-muted-foreground font-mono">{b.filename}</p>}
                  </div>
                ))}
              </div>
            </div>
          )}
          {outcome.healthReport.warnings.length > 0 && (
            <div className="space-y-2">
              <h4 className="text-sm font-semibold">Warnings ({outcome.healthReport.warnings.length})</h4>
              <div className="max-h-32 space-y-1 overflow-y-auto pr-1">
                {outcome.healthReport.warnings.map((w, i) => (
                  <p key={i} className="rounded bg-amber-500/10 p-2 text-xs text-amber-700 dark:text-amber-300">{w.message}</p>
                ))}
              </div>
            </div>
          )}
          <p className="text-xs text-muted-foreground">Snapshot <span className="font-mono">{outcome.snapshotId}</span> can restore the pre-install state at any time.</p>
        </div>
      )}
      {outcome.type === 'failed' && (
        <>
          <div className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">{outcome.error}</div>
          {outcome.rollbackPerformed && (
            <p className="text-xs text-muted-foreground">The recovery snapshot was restored automatically.</p>
          )}
          {outcome.snapshotId && !outcome.rollbackPerformed && (
            <p className="text-xs text-muted-foreground">Snapshot {outcome.snapshotId} is available for recovery.</p>
          )}
        </>
      )}
      {outcome.type === 'cancelled' && (
        <div className="rounded-lg bg-muted p-3 text-sm text-muted-foreground">
          The operation was cancelled before live instance changes were committed.
        </div>
      )}
      {rollbackState === 'restored' && (
        <div className="rounded-lg bg-green-500/10 p-3 text-sm text-green-700 dark:text-green-300">
          The recovery snapshot was restored.
        </div>
      )}
      {rollbackError && (
        <div className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">
          Restore failed: {rollbackError}
        </div>
      )}
      <div className="flex justify-end gap-2">
        <button onClick={onClose} className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent">
          Close
        </button>
        {canRestore && (
          <button
            onClick={() => { void rollback(); }}
            disabled={rollbackState === 'restoring'}
            className="rounded-lg border border-destructive/40 px-4 py-2 text-sm font-medium text-destructive hover:bg-destructive/10 disabled:opacity-50"
          >
            {rollbackState === 'restoring' ? 'Restoring…' : 'Roll Back'}
          </button>
        )}
        {(outcome.type === 'success' || outcome.type === 'health-rollback' || rollbackState === 'restored') && (
          <button onClick={onOpenInstance} data-tour="install-open-instance" className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90">
            Open Instance
          </button>
        )}
      </div>
    </div>
  );
}

function ErrorView({ message, retryable, onRetry, canTryClosest, onTryClosest, canSkip, onSkip, onClose }: {
  message: string;
  retryable: boolean;
  onRetry: () => void;
  canTryClosest: boolean;
  onTryClosest: () => void;
  canSkip: boolean;
  onSkip: () => void;
  onClose: () => void;
}) {
  return (
    <div className="space-y-4 py-4">
      <div className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">{message}</div>
      <div className="flex justify-end gap-2">
        <button onClick={onClose} className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent">Close</button>
        {canTryClosest && (
          <button onClick={onTryClosest} className="rounded-lg border border-primary px-4 py-2 text-sm font-medium text-primary hover:bg-primary/10">
            Try Closest Version
          </button>
        )}
        {canSkip && (
          <button onClick={onSkip} className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent">
            Skip This Mod
          </button>
        )}
        {retryable && <button onClick={onRetry} className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90">Retry</button>}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatBytes(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)} KB`;
  return `${bytes} B`;
}
