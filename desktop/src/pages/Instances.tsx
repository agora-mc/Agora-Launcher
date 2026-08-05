import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  cancelJavaRuntime,
  checkInstanceUpdates,
  cloneInstance,
  createInstance,
  createSnapshot,
  deleteInstance,
  getSetting,
  getCustomIcon,
  getInstanceDetail,
  listInstances,
  listLoaderVersions,
  listManifestLoaders,
  listManifestMcVersions,
  formatError,
  type ClonePrefs,
  type CreateInstanceRequest,
  type InstanceRow,
  type JavaRuntimeProgressEvent,
  type LauncherAction,
  type LoaderVersionSummary,
  type HealthReport,
  type RecoverableJavaIssue,
  type RecoverableProfileIssue,
  type UpdateInfo,
} from '../lib/tauri';
import type { InstallIntent } from '../lib/installFlow';
import { type ProcessState } from '../lib/useProcessController';
import { InstallFlow } from '../components/InstallFlow';
import { formatInstalledDate } from '../components/installed-content/contentTableState';
import { LauncherImportWizard } from '../components/LauncherImportWizard';
import { PackInstallProgressBar, usePackInstall, type PackInstallTask } from '../components/PackInstallProgress';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog';

export function Instances({
  onEditInstance,
  processState,
  onStartLaunch,
  onKillProcess,
  onStartCrashInvestigation,
  onRepairAndRetry,
  onUseDelegatedLaunch,
  onClearError,
  healthReports,
  healthErrors,
  onReviewHealth,
  onRefreshHealth,
}: {
  onEditInstance: (id: string) => void;
  processState: ProcessState;
  onStartLaunch: (instanceId: string, directLaunch: boolean) => Promise<boolean>;
  onKillProcess: () => Promise<void>;
  onStartCrashInvestigation: (investigation: {
    instanceId: string;
    crashFilename: string | null;
    manualLogText: string | null;
    directLaunch: boolean;
  }) => void;
  onRepairAndRetry: () => Promise<void>;
  onUseDelegatedLaunch: () => Promise<void>;
  onClearError: () => void;
  healthReports: Record<string, HealthReport>;
  healthErrors: Record<string, string>;
  onReviewHealth: (instanceId: string, instanceName: string, report: HealthReport) => void;
  onRefreshHealth: () => Promise<void>;
}) {
  const [instances, setInstances] = useState<InstanceRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [instanceIcons, setInstanceIcons] = useState<Record<string, string>>({});
  const [snapshotReadiness, setSnapshotReadiness] = useState<Record<string, 'ready' | 'pending' | 'failed'>>({});
  const [snapshotErrors, setSnapshotErrors] = useState<Record<string, string>>({});
  const [showCreate, setShowCreate] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [cloneTarget, setCloneTarget] = useState<InstanceRow | null>(null);
  const { getTaskForInstance, revision: packInstallRevision } = usePackInstall();

  // Load direct launch mode once
  const [directLaunch, setDirectLaunch] = useState(false);

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      const rows = await listInstances();
      setInstances(rows);
      const details = await Promise.all(rows.map(async (row) => {
        try {
          const detail = await getInstanceDetail(row.instance_id);
          const storedUrl = detail?.manifest?.user_preferences?.agora_pack_icon_url;
          const remoteUrl = typeof storedUrl === 'string' && storedUrl.startsWith('https://')
            ? storedUrl
            : null;
          const customUrl = row.icon_path
            ? await getCustomIcon(row.instance_id, 'instance')
            : null;
          return {
            instanceId: row.instance_id,
            icon: customUrl ?? remoteUrl,
            readiness: detail?.snapshot_readiness ?? 'ready',
            error: detail?.snapshot_error ?? null,
          };
        } catch {
          return { instanceId: row.instance_id, icon: null, readiness: 'ready' as const, error: null };
        }
      }));
      setInstanceIcons(Object.fromEntries(
        details
          .filter((entry): entry is typeof entry & { icon: string } => entry.icon !== null)
          .map((entry) => [entry.instanceId, entry.icon]),
      ));
      setSnapshotReadiness(Object.fromEntries(details.map((entry) => [entry.instanceId, entry.readiness])));
      setSnapshotErrors(Object.fromEntries(
        details.filter((entry) => entry.error !== null).map((entry) => [entry.instanceId, entry.error as string]),
      ));
    } catch (e) {
      setError(formatError(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  useEffect(() => {
    if (packInstallRevision > 0) void refresh();
  }, [packInstallRevision]);

  useEffect(() => {
    if (!Object.values(snapshotReadiness).some((state) => state === 'pending')) return undefined;
    const timer = window.setInterval(() => { void refresh(); }, 1500);
    return () => window.clearInterval(timer);
  }, [snapshotReadiness]);

  // Load launch mode setting on mount
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const mode = await getSetting('launch_mode');
        if (!cancelled) setDirectLaunch(mode === 'direct');
      } catch {
        // Default to delegation
      }
    })();
    return () => { cancelled = true; };
  }, []);

  const openCrashInvestigator = (instanceId: string) => {
    onStartCrashInvestigation({
      instanceId,
      crashFilename: null,
      manualLogText: null,
      directLaunch,
    });
  };

  return (
    <div className="space-y-6">
      <section className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h2 className="text-2xl font-bold mb-2">My Instances</h2>
          <p className="text-muted-foreground">
            Isolated modpack profiles, custom instances, and launch history.
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            onClick={() => setShowImport(true)}
            className="rounded-lg border border-border bg-card px-4 py-2 text-sm font-medium hover:bg-accent"
          >
            Import Instances
          </button>
          <button
            onClick={() => setShowCreate(true)}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
          >
            + Create Instance
          </button>
        </div>
      </section>

      {error && (
        <div className="rounded-lg bg-destructive p-3 text-sm text-destructive-foreground">
          {error}
        </div>
      )}

      {loading ? (
        <div className="rounded-xl border border-dashed border-border bg-card p-6 text-center text-muted-foreground">
          Loading instances…
        </div>
      ) : instances.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border bg-card p-6 text-center">
          <p className="text-muted-foreground">No instances yet.</p>
          <p className="text-sm text-muted-foreground mt-2">
            Create a custom instance to install a verified modloader and launch via the official Mojang launcher or the in-app direct launcher.
          </p>
        </div>
      ) : (
        <ul className="grid grid-cols-1 gap-4 md:grid-cols-2">
          {instances.map((instance) => {
            const isRunning = processState.instanceId === instance.instance_id && processState.phase === 'running';
            const isCurrentFailed = processState.instanceId === instance.instance_id && processState.phase === 'failed';
            const isLaunchBusy = processState.phase === 'launching';
            const isCurrentLaunchBusy = isLaunchBusy && processState.instanceId === instance.instance_id;

            const isCurrentThisInstance = processState.instanceId === instance.instance_id;
            const packInstall = getTaskForInstance(instance.instance_id);
            const recoveryPending = snapshotReadiness[instance.instance_id] === 'pending';
            const recoveryFailed = snapshotReadiness[instance.instance_id] === 'failed';

            return (
              <InstanceCard
                key={instance.instance_id}
                instance={instance}
                iconSrc={instanceIcons[instance.instance_id] ?? null}
                onChanged={refresh}
                onEdit={() => onEditInstance(instance.instance_id)}
                onClone={() => setCloneTarget(instance)}
                onOpenCrashInvestigator={openCrashInvestigator}
                isRunning={isRunning}
                runningPid={isRunning ? processState.pid : null}
                launchBusy={isLaunchBusy}
                onLaunch={() => onStartLaunch(
                  instance.instance_id,
                  instance.launch_mode_override === 'direct'
                    || (instance.launch_mode_override !== 'delegated' && directLaunch),
                )}
                onKill={onKillProcess}
                controllerError={isCurrentFailed ? processState.error : null}
                controllerRecoverableIssue={isCurrentFailed ? processState.recoverableIssue : null}
                controllerRecoverableJavaIssue={isCurrentThisInstance ? processState.recoverableJavaIssue : null}
                controllerAvailableActions={isCurrentFailed ? processState.availableActions : []}
                runtimeProgress={isCurrentThisInstance ? processState.runtimeProgress : null}
                onDismissError={onClearError}
                onRepairAndRetry={onRepairAndRetry}
                onUseDelegatedLaunch={onUseDelegatedLaunch}
                repairBusy={isCurrentLaunchBusy}
                packInstall={packInstall}
                recoveryPending={recoveryPending}
                recoveryError={snapshotErrors[instance.instance_id] ?? null}
                recoveryFailed={recoveryFailed}
                healthReport={healthReports[instance.instance_id] ?? null}
                healthError={healthErrors[instance.instance_id] ?? null}
                onReviewHealth={onReviewHealth}
                onRefreshHealth={onRefreshHealth}
                onRetryRecovery={async () => {
                  await createSnapshot(instance.instance_id, 'Initial import retry');
                  await refresh();
                }}
              />
            );
          })}
        </ul>
      )}

      {instances.length > 0 && (
        <UpdatesSection instances={instances} />
      )}

      {showCreate && (
        <CreateInstanceDialog
          onClose={() => setShowCreate(false)}
          onCreated={() => {
            setShowCreate(false);
            refresh();
          }}
        />
      )}

      {cloneTarget && (
        <CloneInstanceDialog
          instance={cloneTarget}
          onClose={() => setCloneTarget(null)}
          onCloned={() => {
            setCloneTarget(null);
            refresh();
          }}
        />
      )}

      <LauncherImportWizard
        open={showImport}
        onClose={() => setShowImport(false)}
        onComplete={refresh}
      />

    </div>
  );
}
// ─── Instance Card with recoverable profile warning panel ─────────────────

const PROFILE_ISSUE_MESSAGES: Record<string, { title: string; description: string }> = {
  UnsupportedProfileMetadata: {
    title: 'Unsupported Profile',
    description:
      'Agora Direct Launch does not understand part of this loader profile. This may be a newer profile format or a damaged installation.',
  },
  CorruptProfile: {
    title: 'Corrupted Profile',
    description:
      'This loader profile failed integrity or safety checks and may be corrupted.',
  },
  MissingProfile: {
    title: 'Missing Profile',
    description:
      'The installed loader profile is missing.',
  },
};

function InstanceCard({
  instance,
  iconSrc,
  onChanged,
  onEdit,
  onClone,
  onOpenCrashInvestigator,
  isRunning,
  runningPid,
  launchBusy,
  onLaunch,
  onKill,
  controllerError,
  controllerRecoverableIssue,
  controllerRecoverableJavaIssue,
  controllerAvailableActions,
  runtimeProgress,
  onDismissError,
  onRepairAndRetry,
  onUseDelegatedLaunch,
  repairBusy,
  packInstall,
  recoveryPending,
  recoveryError,
  recoveryFailed,
  healthReport,
  healthError,
  onReviewHealth,
  onRefreshHealth,
  onRetryRecovery,
}: {
  instance: InstanceRow;
  iconSrc: string | null;
  onChanged: () => void;
  onEdit: () => void;
  onClone: () => void;
  onOpenCrashInvestigator: (id: string) => void;
  isRunning: boolean;
  runningPid: number | null;
  launchBusy: boolean;
  onLaunch: () => void;
  onKill: () => void;
  controllerError: string | null;
  controllerRecoverableIssue: RecoverableProfileIssue | null;
  controllerRecoverableJavaIssue: RecoverableJavaIssue | null;
  controllerAvailableActions: LauncherAction[];
  runtimeProgress: JavaRuntimeProgressEvent | null;
  onDismissError: () => void;
  onRepairAndRetry: () => Promise<void>;
  onUseDelegatedLaunch: () => Promise<void>;
  repairBusy: boolean;
  packInstall: PackInstallTask | null;
  recoveryPending: boolean;
  recoveryError: string | null;
  recoveryFailed: boolean;
  healthReport: HealthReport | null;
  healthError: string | null;
  onReviewHealth: (instanceId: string, instanceName: string, report: HealthReport) => void;
  onRefreshHealth: () => Promise<void>;
  onRetryRecovery: () => Promise<void>;
}) {
  const [error, setError] = useState<string | null>(null);
  const [repairing, setRepairing] = useState(false);
  const [cancellingJava, setCancellingJava] = useState(false);
  const [retryingRecovery, setRetryingRecovery] = useState(false);
  const [refreshingHealth, setRefreshingHealth] = useState(false);

  const displayError = error ?? controllerError;
  const effectiveBusy = launchBusy || repairBusy || repairing || cancellingJava;
  const recoveryBlocked = recoveryPending || recoveryFailed || packInstall?.status === 'running';
  const healthIssueCount = healthReport
    ? healthReport.blockers.length + healthReport.warnings.length
    : 0;
  const healthBlocked = (healthReport?.blockers.length ?? 0) > 0;

  const handleCancelJavaProvisioning = async () => {
    if (!runtimeProgress) return;
    setCancellingJava(true);
    try {
      await cancelJavaRuntime(`java-runtime-${instance.instance_id}-${runtimeProgress.major}`);
    } catch {
      // Operation may already be complete — ignore
    }
  };

  const handleRetryRecovery = async () => {
    setRetryingRecovery(true);
    setError(null);
    try {
      await onRetryRecovery();
    } catch (cause) {
      setError(formatError(cause));
    } finally {
      setRetryingRecovery(false);
    }
  };

  const handleRefreshHealth = async () => {
    setRefreshingHealth(true);
    try {
      await onRefreshHealth();
    } finally {
      setRefreshingHealth(false);
    }
  };

  const remove = async () => {
    if (!confirm(`Delete instance "${instance.name}"? This moves the folder to trash.`)) return;
    setError(null);
    try {
      await deleteInstance(instance.instance_id);
      onChanged();
    } catch (e) {
      setError(formatError(e));
    }
  };

  const handleReinstall = async () => {
    setRepairing(true);
    try {
      await onRepairAndRetry();
    } catch {
      // Error state is already managed by the controller.
    } finally {
      setRepairing(false);
    }
  };

  const handleDelegatedLaunch = async () => {
    setRepairing(true);
    try {
      await onUseDelegatedLaunch();
    } catch {
      // Error state is already managed by the controller.
    } finally {
      setRepairing(false);
    }
  };

  const issueDef = controllerRecoverableIssue
    ? PROFILE_ISSUE_MESSAGES[controllerRecoverableIssue.kind]
    : null;

  return (
    <li className="rounded-xl border border-border bg-card p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-3">
          {iconSrc && (
            <img
              src={iconSrc}
              alt={`${instance.name} icon`}
              className="h-14 w-14 shrink-0 rounded-xl border border-border object-cover"
            />
          )}
          <div className="min-w-0">
            <h3 className="break-words font-semibold">{instance.name}</h3>
            {instance.import_source && (
              <span className="mt-1 inline-flex rounded-full bg-muted px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                Imported from {instance.import_source === 'curse_forge' || instance.import_source === 'curseforge'
                  ? 'CurseForge'
                  : instance.import_source === 'prism' ? 'Prism Launcher' : 'Modrinth App'}
              </span>
            )}
            <p className="text-xs text-muted-foreground">
              {instance.loader === 'vanilla' || !instance.loader_version
                ? instance.loader
                : `${instance.loader} ${instance.loader_version}`}
              {' '}· MC {instance.minecraft_version}
            </p>
            <p className="text-xs text-muted-foreground mt-1">
              {isRunning ? (
                <span className="text-green-600 dark:text-green-400">● Running (PID {runningPid})</span>
              ) : instance.last_launched_at ? (
                `Last launched ${formatInstalledDate(instance.last_launched_at)}`
              ) : (
                'Never launched'
              )}
            </p>
          </div>
        </div>
        <span className="text-xs uppercase tracking-wide text-muted-foreground">
          {instance.launch_mode_override === 'delegated'
            ? 'Delegated only'
            : instance.is_locked ? 'Locked' : 'Unlocked'}
        </span>
      </div>

      {packInstall && <PackInstallProgressBar task={packInstall} compact />}

      {healthReport && healthIssueCount > 0 && (
        <div
          className={`mt-3 flex items-center justify-between gap-3 rounded-lg border px-3 py-2 text-xs ${
            healthBlocked
              ? 'border-destructive/60 bg-destructive/10'
              : 'border-amber-500/60 bg-amber-500/10'
          }`}
          role="alert"
          aria-label={`${healthIssueCount} health issue${healthIssueCount === 1 ? '' : 's'} detected`}
        >
          <div className="min-w-0">
            <p className={healthBlocked ? 'font-medium text-destructive' : 'font-medium text-amber-700 dark:text-amber-300'}>
              {healthIssueCount} health issue{healthIssueCount === 1 ? '' : 's'} detected
            </p>
            <p className="truncate text-muted-foreground">
              {healthBlocked
                ? 'Review before launching. You can continue after confirming risk.'
                : 'Review recommended before launch.'}
            </p>
          </div>
          <button
            type="button"
            onClick={() => onReviewHealth(instance.instance_id, instance.name, healthReport)}
            className="shrink-0 rounded border border-current/30 px-2 py-1 font-medium hover:bg-background/40"
          >
            Review & repair
          </button>
        </div>
      )}

      {healthError && (
        <div
          className="mt-3 flex items-center justify-between gap-3 rounded-lg border border-amber-500/60 bg-amber-500/10 px-3 py-2 text-xs"
          role="alert"
          aria-label="Health check unavailable"
        >
          <div className="min-w-0">
            <p className="font-medium text-amber-700 dark:text-amber-300">Health check unavailable</p>
            <p className="truncate text-muted-foreground">{healthError}</p>
          </div>
          <button
            type="button"
            onClick={() => void handleRefreshHealth()}
            disabled={refreshingHealth}
            className="shrink-0 rounded border border-current/30 px-2 py-1 font-medium hover:bg-background/40 disabled:opacity-50"
          >
            {refreshingHealth ? 'Checking…' : 'Check again'}
          </button>
        </div>
      )}

      {recoveryBlocked && (
        <div className="mt-3 rounded-lg border border-amber-500 bg-amber-500/10 p-3 text-xs" role="status">
          <p className="font-medium text-amber-700 dark:text-amber-300">
            {recoveryPending ? 'Finalizing recovery snapshot…' : 'Recovery snapshot failed'}
          </p>
          <p className="mt-1 text-muted-foreground">
            {recoveryPending
              ? 'This instance is available for inspection, but launching and changes are temporarily disabled.'
              : recoveryError ?? 'The instance is protected from changes until a recovery snapshot is available.'}
          </p>
          {recoveryFailed && (
            <button
              type="button"
              onClick={handleRetryRecovery}
              disabled={retryingRecovery}
              className="mt-2 rounded-lg border border-amber-500/50 px-3 py-1.5 font-medium hover:bg-amber-500/10 disabled:opacity-50"
            >
              {retryingRecovery ? 'Retrying snapshot…' : 'Retry recovery snapshot'}
            </button>
          )}
        </div>
      )}

      {/* ── Recoverable profile warning panel ── */}
      {controllerRecoverableIssue && issueDef && (
        <div
          className="mt-3 rounded-lg border border-amber-500 bg-amber-500/10 p-3 space-y-2"
          role="alert"
          aria-label={`Profile issue: ${issueDef.title}`}
          data-testid="recoverable-profile-warning"
        >
          <div className="flex items-start justify-between gap-2">
            <div>
              <p className="text-sm font-semibold text-amber-700 dark:text-amber-300">
                {issueDef.title}
              </p>
              <p className="text-xs text-muted-foreground mt-0.5">
                {issueDef.description}
              </p>
            </div>
          </div>

          {/* Render reasons (max 5) */}
          {controllerRecoverableIssue.reasons.length > 0 && (
            <ul className="text-xs text-muted-foreground space-y-0.5 list-disc list-inside">
              {controllerRecoverableIssue.reasons.slice(0, 5).map((reason, i) => (
                <li key={i}>{reason}</li>
              ))}
              {controllerRecoverableIssue.reasons.length > 5 && (
                <li className="text-[10px] italic">… and {controllerRecoverableIssue.reasons.length - 5} more</li>
              )}
            </ul>
          )}

          {/* Action buttons based on availableActions */}
          <div className="flex flex-wrap gap-2 mt-2">
            {controllerAvailableActions.includes('reinstall_loader') && (
              <button
                onClick={handleReinstall}
                disabled={effectiveBusy || recoveryBlocked}
                aria-label="Reinstall loader and retry launch"
                className="rounded-lg bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              >
                {repairing ? 'Reinstalling loader…' : 'Reinstall loader'}
              </button>
            )}
            {controllerAvailableActions.includes('use_delegated_launch') && (
              <button
                onClick={handleDelegatedLaunch}
                disabled={effectiveBusy || recoveryBlocked}
                aria-label="Use delegated launch"
                className="rounded-lg border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50"
              >
                {repairing ? 'Launching via Mojang…' : 'Use delegated launch'}
              </button>
            )}
            {controllerAvailableActions.includes('dismiss') && (
              <button
                onClick={onDismissError}
                disabled={effectiveBusy || recoveryBlocked}
                aria-label="Dismiss this error"
                className="rounded-lg border border-border px-3 py-1.5 text-xs font-medium text-muted-foreground hover:bg-accent disabled:opacity-50"
              >
                Dismiss
              </button>
            )}
          </div>

          {repairing && (
            <p className="text-xs text-muted-foreground italic" aria-live="polite">
              {controllerAvailableActions.includes('reinstall_loader') && !controllerAvailableActions.includes('use_delegated_launch')
                ? 'Reinstalling loader…'
                : 'Processing…'}
            </p>
          )}
        </div>
      )}

      {/* ── Java runtime provisioning panel ── */}
      {runtimeProgress && !controllerRecoverableJavaIssue && (
        <div className="mt-3 rounded-lg border border-blue-500 bg-blue-500/10 p-3 space-y-2">
          <div className="flex items-center justify-between gap-2">
            <div className="flex-1 min-w-0">
              <p className="text-sm font-semibold text-blue-700 dark:text-blue-300">
                Provisioning Java {runtimeProgress.major}…
              </p>
              <p className="text-xs text-muted-foreground mt-0.5 truncate">
                {runtimeProgress.message}
              </p>
            </div>
            <button
              onClick={handleCancelJavaProvisioning}
              disabled={cancellingJava}
              className="rounded-lg border border-border px-2.5 py-1 text-xs font-medium hover:bg-accent disabled:opacity-50 shrink-0"
            >
              {cancellingJava ? 'Cancelling…' : 'Cancel'}
            </button>
          </div>
          <div className="h-1.5 bg-background rounded-full overflow-hidden">
            <div
              className="h-full bg-blue-500 rounded-full transition-all duration-300"
              style={{ width: `${Math.min(runtimeProgress.percent, 100)}%` }}
            />
          </div>
        </div>
      )}

      {/* ── Java runtime cancelled panel ── */}
      {controllerRecoverableJavaIssue && controllerAvailableActions.includes('cancel') && !controllerAvailableActions.includes('download_runtime') && !controllerAvailableActions.includes('choose_java') && !controllerAvailableActions.includes('open_privacy') && (
        <div className="mt-3 rounded-lg border border-amber-500 bg-amber-500/10 p-3">
          <p className="text-sm font-semibold text-amber-700 dark:text-amber-300">
            Java provisioning cancelled
          </p>
          <p className="text-xs text-muted-foreground mt-0.5">
            Java {controllerRecoverableJavaIssue.major} runtime download was cancelled.
          </p>
        </div>
      )}

      {/* ── Java runtime download disabled panel ── */}
      {controllerRecoverableJavaIssue && controllerAvailableActions.includes('open_privacy') && (
        <div className="mt-3 rounded-lg border border-amber-500 bg-amber-500/10 p-3 space-y-2">
          <p className="text-sm font-semibold text-amber-700 dark:text-amber-300">
            Java downloads are disabled
          </p>
          <p className="text-xs text-muted-foreground">
            Java {controllerRecoverableJavaIssue.major} runtime download is disabled in Privacy settings.
            Enable "Java runtime downloads" or choose a local Java installation.
          </p>
          <div className="flex flex-wrap gap-2 mt-1">
            {controllerAvailableActions.includes('choose_java') && (
              <button
                onClick={onDismissError}
                className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90"
              >
                Choose Java…
              </button>
            )}
            <button
              onClick={() => {
                window.dispatchEvent(new CustomEvent('agora-navigate', { detail: 'settings' }));
                onDismissError();
              }}
              className="rounded-lg border border-border px-3 py-1.5 text-xs font-medium hover:bg-accent"
            >
              Open Privacy Settings
            </button>
          </div>
        </div>
      )}

      {/* ── Plain error display (fallback, non-recoverable) ── */}
      {displayError && !controllerRecoverableIssue && !controllerRecoverableJavaIssue && (
        <div className="mt-2 flex items-center gap-2">
          <p className="text-xs text-destructive flex-1">{displayError}</p>
          {controllerError && (
            <button
              onClick={onDismissError}
              className="text-xs text-muted-foreground hover:underline"
            >
              Dismiss
            </button>
          )}
        </div>
      )}

      <div className="mt-4 flex flex-wrap gap-2">
        {isRunning ? (
          <button
            onClick={onKill}
            className="rounded-lg bg-destructive px-3 py-1.5 text-sm font-medium text-destructive-foreground hover:bg-destructive/90"
          >
            Kill
          </button>
        ) : (
          <button
            onClick={onLaunch}
            disabled={effectiveBusy || recoveryBlocked}
            className="rounded-lg bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {effectiveBusy && !repairing ? 'Starting…' : 'Launch'}
          </button>
        )}
        <button
          onClick={onEdit}
          disabled={effectiveBusy || recoveryBlocked}
          className="rounded-lg border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50"
        >
          Edit
        </button>
        <button
          onClick={onClone}
          disabled={effectiveBusy || recoveryBlocked || instance.is_locked}
          className="rounded-lg border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50"
        >
          Clone
        </button>
        <button
          onClick={() => onOpenCrashInvestigator(instance.instance_id)}
          disabled={effectiveBusy || recoveryBlocked}
          className="rounded-lg border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50"
        >
          Troubleshoot
        </button>
        <button
          onClick={remove}
          disabled={effectiveBusy || recoveryBlocked}
          className="rounded-lg border border-destructive/30 px-3 py-1.5 text-sm font-medium text-destructive hover:bg-destructive/10 disabled:opacity-50"
        >
          Delete
        </button>
      </div>
    </li>
  );
}

/** A section that checks for updates, batches them, and applies them safely. */

function UpdatesSection({
  instances,
}: {
  instances: InstanceRow[];
}) {
  const [updatesByInstance, setUpdatesByInstance] = useState<Record<string, UpdateInfo[]>>({});
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [checking, setChecking] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [batchFlow, setBatchFlow] = useState<{
    intent: InstallIntent;
    instanceName: string;
  } | null>(null);
  const [showConfirm, setShowConfirm] = useState<{
    instanceId: string;
    instanceName: string;
    updates: UpdateInfo[];
  } | null>(null);

  const checkAll = async () => {
    setChecking(true);
    setUpdateError(null);
    const results: Record<string, UpdateInfo[]> = {};
    let failedChecks = 0;
    for (const inst of instances) {
      if (inst.is_locked) continue; // skip locked instances
      try {
        const updates = await checkInstanceUpdates(inst.instance_id);
        if (updates.length > 0) results[inst.instance_id] = updates;
      } catch {
        failedChecks += 1;
      }
    }
    setUpdatesByInstance(results);
    setSelected(new Set());
    if (failedChecks > 0) {
      setUpdateError(`Could not check ${failedChecks} instance${failedChecks === 1 ? '' : 's'} for updates.`);
    }
    setChecking(false);
  };

  const totalUpdates = Object.values(updatesByInstance).reduce((sum, u) => sum + u.length, 0);

  /** Toggle per-mod selection. */
  const toggleSelected = (key: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key); else next.add(key);
      return next;
    });
  };

  const applyUpdates = () => {
    if (!showConfirm) return;

    const { instanceId, instanceName, updates } = showConfirm;
    const toUpdate = selected.size > 0
      ? updates.filter((update) => selected.has(`${instanceId}:${update.mod_jar_id}`))
      : updates;
    if (toUpdate.length === 0) return;

    setShowConfirm(null);
    setUpdateError(null);
    setBatchFlow({
      instanceName,
      intent: {
        action: {
          type: 'batch-update',
          items: toUpdate.map((update) => ({
            itemId: update.mod_jar_id,
            targetVersion: update.target_version,
          })),
        },
        targetInstance: instanceId,
        optionalDeps: { type: 'prompt' },
        requestedBy: 'auto-update',
        overrides: {
          allowReplace: false,
          skipHealthScan: false,
          forceConflictResolution: {},
        },
      },
    });
  };

  if (totalUpdates === 0 && !checking) {
    return (
      <div className="mt-6">
        <button
          onClick={checkAll}
          disabled={checking}
          className="rounded-lg border border-border px-4 py-2 text-sm font-medium hover:bg-accent disabled:opacity-50"
        >
          {checking ? 'Checking…' : 'Check for Updates'}
        </button>
      </div>
    );
  }

  const allSelected = (updates: UpdateInfo[], instId: string) =>
    updates.every((u) => selected.has(`${instId}:${u.mod_jar_id}`));

  return (
    <div className="mt-6 space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="font-semibold">Updates Available ({totalUpdates})</h3>
        <button
          onClick={checkAll}
          disabled={checking}
          className="rounded-lg border border-border px-3 py-1.5 text-xs font-medium hover:bg-accent disabled:opacity-50"
        >
          {checking ? 'Checking…' : 'Refresh'}
        </button>
      </div>
      {updateError && (
        <div className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">
          {updateError}
        </div>
      )}
      {Object.entries(updatesByInstance).map(([instId, updates]) => {
        const inst = instances.find((i) => i.instance_id === instId);
        const locked = inst?.is_locked ?? false;
        const selectedCount = updates.filter((u) => selected.has(`${instId}:${u.mod_jar_id}`)).length;
        return (
          <div key={instId} className="rounded-xl border border-border bg-card p-4 space-y-3">
            <div className="flex items-center justify-between">
              <p className="text-sm font-medium">{inst?.name ?? instId}</p>
              {locked && <span className="text-xs text-muted-foreground">🔒 Locked — updates disabled</span>}
            </div>
            <div className="space-y-1">
              {updates.map((u) => {
                const key = `${instId}:${u.mod_jar_id}`;
                return (
                  <div key={u.mod_jar_id} className="flex items-center gap-2 text-xs">
                    {!locked && (
                      <input
                        type="checkbox"
                        checked={selected.has(key)}
                        onChange={() => toggleSelected(key)}
                        className="rounded"
                      />
                    )}
                    <span className="flex-1">{u.filename}</span>
                    <span className="text-muted-foreground">{u.current_version} → <span className="text-primary">{u.latest_version}</span></span>
                  </div>
                );
              })}
            </div>
            {!locked && (
              <div className="flex gap-2">
                <button
                  onClick={() => {
                    // Select/deselect all for this instance
                    setSelected((previous) => {
                      const next = new Set(previous);
                      if (updates.every((update) => next.has(`${instId}:${update.mod_jar_id}`))) {
                        updates.forEach((update) => next.delete(`${instId}:${update.mod_jar_id}`));
                      } else {
                        updates.forEach((update) => next.add(`${instId}:${update.mod_jar_id}`));
                      }
                      return next;
                    });
                  }}
                  className="text-xs text-primary hover:underline"
                >
                  {allSelected(updates, instId) ? 'Deselect all' : 'Select all'}
                </button>
                {selectedCount > 0 && (
                  <button
                    onClick={() => setShowConfirm({ instanceId: instId, instanceName: inst?.name ?? instId, updates })}
                    className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
                  >
                    Update Selected ({selectedCount})
                  </button>
                )}
                {selectedCount === 0 && (
                  <button
                    onClick={() => {
                      setSelected((previous) => {
                        const next = new Set(previous);
                        updates.forEach((update) => next.add(`${instId}:${update.mod_jar_id}`));
                        return next;
                      });
                      setShowConfirm({ instanceId: instId, instanceName: inst?.name ?? instId, updates });
                    }}
                    className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
                  >
                    Update All ({updates.length})
                  </button>
                )}
              </div>
            )}
          </div>
        );
      })}

      <Dialog open={showConfirm !== null} onOpenChange={(open) => { if (!open) setShowConfirm(null); }}>
        {showConfirm && (
          <DialogContent>
            <DialogTitle>
              Review {showConfirm.updates.filter((update) => selected.has(`${showConfirm.instanceId}:${update.mod_jar_id}`) || selected.size === 0).length} updates
            </DialogTitle>
            <DialogDescription>
              Agora will resolve dependencies and conflicts for {showConfirm.instanceName} before anything changes.
            </DialogDescription>
            <ul className="max-h-48 space-y-1 overflow-y-auto text-xs">
              {showConfirm.updates
                .filter((update) => selected.has(`${showConfirm.instanceId}:${update.mod_jar_id}`) || selected.size === 0)
                .map((update) => (
                  <li key={update.mod_jar_id} className="flex justify-between gap-4">
                    <span className="truncate">{update.filename}</span>
                    <span className="shrink-0 text-muted-foreground">
                      {update.current_version} → <span className="text-primary">{update.latest_version}</span>
                    </span>
                  </li>
                ))}
            </ul>
            <p className="text-xs text-muted-foreground">
              The complete batch is staged and verified first, then applied atomically behind one recovery snapshot.
            </p>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setShowConfirm(null)}
                className="rounded-lg border border-border px-4 py-2 text-sm font-medium hover:bg-accent"
              >
                Cancel
              </button>
              <button
                onClick={applyUpdates}
                className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
              >
                Review Plan
              </button>
            </div>
          </DialogContent>
        )}
      </Dialog>

      {batchFlow && (
        <InstallFlow
          open
          intent={batchFlow.intent}
          instanceName={batchFlow.instanceName}
          onClose={() => {
            setBatchFlow(null);
            void checkAll();
          }}
        />
      )}
    </div>
  );
}

function CreateInstanceDialog({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: () => void;
}) {
  const [name, setName] = useState('');
  const [mcVersion, setMcVersion] = useState('');
  const [loader, setLoader] = useState('fabric');
  const [loaderVersions, setLoaderVersions] = useState<LoaderVersionSummary[]>([]);
  const [loaderVersion, setLoaderVersion] = useState('');
  const [memoryMb, setMemoryMb] = useState(4096);
  const [memoryMode, setMemoryMode] = useState<'auto' | 'manual'>('auto');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [progressMessage, setProgressMessage] = useState<string | null>(null);
  const [loaders, setLoaders] = useState<string[]>([]);
  const [mcVersions, setMcVersions] = useState<string[]>([]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const versions = await listLoaderVersions(loader, mcVersion);
        if (cancelled) return;
        setLoaderVersions(versions);
        setLoaderVersion(versions[0]?.loader_version ?? '');
      } catch (e) {
        if (!cancelled) setError(formatError(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [loader, mcVersion]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [l, v] = await Promise.all([listManifestLoaders(), listManifestMcVersions()]);
        if (!cancelled) {
          setLoaders(l);
          setMcVersions(v);
          if (!mcVersion && v.length > 0) {
            setMcVersion(v[0]);
          }
        }
      } catch {
        // Fetch failure: dropdowns render empty — acceptable degraded behavior.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Re-filter MC versions when the loader changes.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (!loader) return;
      try {
        const filtered = loader === 'vanilla'
          ? await listManifestMcVersions()
          : await listManifestMcVersions(loader);
        if (cancelled) return;
        if (filtered.length > 0) {
          setMcVersions(filtered);
          if (!filtered.includes(mcVersion)) {
            setMcVersion(filtered[0]);
          }
        }
      } catch {
        // Fetch failure — keep existing list (graceful)
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [loader]);

  // Vanilla instances have no loader to pin; clear any stale version.
  useEffect(() => {
    if (loader === 'vanilla') setLoaderVersion('');
  }, [loader]);

  // Progress event listener during creation
  useEffect(() => {
    if (!busy) return;
    setProgressMessage('Starting…');
    const unlisten = listen<{ instance_id: string; stage: string; message: string }>('instance:create-progress', (e) => {
      setProgressMessage(e.payload.message);
    });
    return () => { unlisten.then(fn => fn()); };
  }, [busy]);

  const submit = async () => {
    setBusy(true);
    setError(null);
    setProgressMessage(null);
    try {
      const instanceId = name
        .toLowerCase()
        .replace(/[^a-z0-9-_]+/g, '-')
        .replace(/^-+|-+$/g, '');
      if (!instanceId) throw new Error('Enter a valid instance name.');
      if (!loaderVersion && loader !== 'vanilla') throw new Error('No pinned loader version selected.');

      const request: CreateInstanceRequest = {
        name,
        instance_id: instanceId,
        minecraft_version: mcVersion,
        loader,
        loader_version: loaderVersion,
        jvm_memory_mb: memoryMb,
        jvm_memory_mode: memoryMode,
      };
      await createInstance(request);
      onCreated();
    } catch (e) {
      setError(formatError(e));
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="max-w-lg">
        <DialogTitle>Create Custom Instance</DialogTitle>
        <DialogDescription>
          {loader === 'vanilla'
            ? 'Set up a new vanilla Minecraft instance with no mod loader.'
            : 'Set up a new isolated modpack profile with a verified modloader.'}
        </DialogDescription>

        <div className="space-y-4">
          <label className="block">
            <span className="text-sm font-medium">Instance name</span>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Optimized Survival"
              className="mt-1 w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
            />
          </label>

          <div className="grid grid-cols-2 gap-4">
            <label className="block">
              <span className="text-sm font-medium">Minecraft version</span>
              <select
                value={mcVersion}
                onChange={(e) => setMcVersion(e.target.value)}
                className="mt-1 w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
              >
                {mcVersions.map((v) => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </select>
            </label>

            <label className="block">
              <span className="text-sm font-medium">Loader</span>
              <select
                value={loader}
                onChange={(e) => setLoader(e.target.value)}
                className="mt-1 w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
              >
                {['vanilla', ...loaders.filter((l) => l !== 'vanilla')].map((l) => (
                  <option key={l} value={l}>
                    {l === 'vanilla' ? 'Vanilla' : l}
                  </option>
                ))}
              </select>
            </label>
          </div>

          {loader !== 'vanilla' && (
            <label className="block">
              <span className="text-sm font-medium">Loader version</span>
              <select
                value={loaderVersion}
                onChange={(e) => setLoaderVersion(e.target.value)}
                className="mt-1 w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
              >
                {loaderVersions.length === 0 && <option value="">No pinned versions</option>}
                {loaderVersions.map((v) => (
                  <option key={v.loader_version} value={v.loader_version}>
                    {v.loader_version} ({v.file_type})
                  </option>
                ))}
              </select>
            </label>
          )}

          <label className="block">
            <span className="flex items-center justify-between text-sm font-medium">
              <span>JVM memory: {memoryMode === 'auto' ? 'Auto' : `${memoryMb} MB`}</span>
              <span className="flex items-center gap-2 text-xs text-muted-foreground">
                Automatic
                <input
                  type="checkbox"
                  checked={memoryMode === 'auto'}
                  onChange={(event) => setMemoryMode(event.target.checked ? 'auto' : 'manual')}
                />
              </span>
            </span>
            <input
              type="range"
              min={1024}
              max={16384}
              step={512}
              value={memoryMb}
              onChange={(e) => setMemoryMb(Number(e.target.value))}
              disabled={memoryMode === 'auto'}
              className="mt-1 w-full accent-primary"
            />
            {memoryMode === 'auto' && (
              <span className="mt-1 block text-xs text-muted-foreground">
                Agora will estimate memory from enabled pack content and system headroom.
              </span>
            )}
          </label>
        </div>

        {progressMessage && (
          <p className="mt-4 text-sm text-muted-foreground">{progressMessage}</p>
        )}

        {error && (
          <p className="mt-4 text-sm text-destructive">{error}</p>
        )}

        <div className="mt-6 flex justify-end gap-2">
          <button
            onClick={onClose}
            disabled={busy}
            className="rounded-lg border border-input bg-background px-4 py-2 text-sm font-medium hover:bg-accent"
          >
            Cancel
          </button>
          <button
            onClick={submit}
            disabled={busy}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy ? 'Creating…' : 'Create'}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

const DEFAULT_CLONE_PREFS: ClonePrefs = {
  copy_saves: true,
  copy_mods: true,
  copy_resource_packs: true,
  copy_shader_packs: true,
  copy_screenshots: true,
  copy_config: true,
  copy_servers: true,
  copy_options: true,
  use_hard_links: false,
  use_sym_links: false,
};

function CloneInstanceDialog({
  instance,
  onClose,
  onCloned,
}: {
  instance: InstanceRow;
  onClose: () => void;
  onCloned: () => void;
}) {
  const [name, setName] = useState(`Copy of ${instance.name}`);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!name.trim()) {
      setError('Enter a name for the clone.');
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await cloneInstance(instance.instance_id, name.trim(), DEFAULT_CLONE_PREFS);
      onCloned();
    } catch (e) {
      setError(formatError(e));
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="max-w-md">
        <DialogTitle>Clone Instance</DialogTitle>
        <DialogDescription>
          Creates a full copy of "{instance.name}" including saves, mods, resource packs,
          shaders, config, and options. The original instance is not modified.
        </DialogDescription>
        <div className="space-y-4">
          <label className="block">
            <span className="text-sm font-medium">New instance name</span>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="mt-1 w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
            />
          </label>
          {error && <p className="text-xs text-destructive">{error}</p>}
        </div>
        <div className="mt-6 flex justify-end gap-2">
          <button
            onClick={onClose}
            disabled={busy}
            className="rounded-lg border border-input bg-background px-4 py-2 text-sm font-medium hover:bg-accent"
          >
            Cancel
          </button>
          <button
            onClick={submit}
            disabled={busy}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy ? 'Cloning…' : 'Clone'}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
