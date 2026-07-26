import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  formatError,
  importInstancePack,
  importModrinthPackByUrl,
} from '../lib/tauri';
import { applyInstallPlan, type InstallOutcome, type ProgressEvent, type ResolvedInstallPlan } from '../lib/installFlow';

export type PackInstallTask = {
  id: string;
  label: string;
  instanceId: string | null;
  instanceName: string;
  kind: 'plan' | 'modrinth-pack' | 'pack-file';
  planId?: string;
  operationId?: string;
  status: 'running' | 'completed' | 'failed';
  phase: string;
  message: string;
  progress: number | null;
  step: number;
  totalSteps: number;
  bytesDownloaded: number;
  bytesTotal: number;
  error: string | null;
};

type PackInstallProgressEvent = {
  operationId: string;
  phase: string;
  message: string;
  progress?: number | null;
  step?: number | null;
  totalSteps?: number | null;
  bytesDownloaded?: number | null;
  bytesTotal?: number | null;
};

type PackInstallContextValue = {
  revision: number;
  tasks: PackInstallTask[];
  getTaskForInstance: (instanceId: string) => PackInstallTask | null;
  startPlan: (plan: ResolvedInstallPlan, label: string, instanceName?: string) => void;
  startModrinthPack: (downloadUrl: string, label: string) => void;
  startPackFile: (sourcePath: string, label: string) => void;
};

const PackInstallContext = createContext<PackInstallContextValue | null>(null);

function initialTask(
  id: string,
  label: string,
  kind: PackInstallTask['kind'],
  instanceId: string | null,
  instanceName: string,
): PackInstallTask {
  return {
    id,
    label,
    instanceId,
    instanceName,
    kind,
    status: 'running',
    phase: 'resolving',
    message: 'Preparing installation…',
    progress: null,
    step: 0,
    totalSteps: 0,
    bytesDownloaded: 0,
    bytesTotal: 0,
    error: null,
  };
}

function updateTask(
  tasks: Record<string, PackInstallTask>,
  id: string,
  update: Partial<PackInstallTask>,
): Record<string, PackInstallTask> {
  const task = tasks[id];
  if (!task) return tasks;
  return { ...tasks, [id]: { ...task, ...update } };
}

function progressFraction(task: PackInstallTask): number | null {
  if (task.progress !== null && Number.isFinite(task.progress)) {
    return Math.max(0, Math.min(1, task.progress));
  }
  if (task.bytesTotal > 0) {
    return Math.max(0, Math.min(1, task.bytesDownloaded / task.bytesTotal));
  }
  if (task.totalSteps > 0) {
    return Math.max(0, Math.min(1, task.step / task.totalSteps));
  }
  return null;
}

function displayPercent(task: PackInstallTask): number | null {
  const fraction = progressFraction(task);
  return fraction === null ? null : Math.round(fraction * 100);
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function phaseLabel(phase: string): string {
  switch (phase) {
    case 'resolving': return 'Preparing installation';
    case 'staging': return 'Loading files';
    case 'downloading': return 'Downloading pack files';
    case 'extracting': return 'Extracting pack files';
    case 'snapshotting': return 'Creating recovery snapshot';
    case 'applying': return 'Applying instance changes';
    case 'health-scan': return 'Checking pack health';
    case 'done': return 'Finishing installation';
    case 'failed': return 'Installation failed';
    default: return 'Installing pack';
  }
}

export function PackInstallProvider({ children }: { children: ReactNode }) {
  const [taskMap, setTaskMap] = useState<Record<string, PackInstallTask>>({});
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    const unlistenPlan = listen<ProgressEvent>('install:progress', (event) => {
      const progress = event.payload;
      const id = progress.planId;
      if (!id) return;
      setTaskMap((current) => {
        const planTask = Object.values(current).find(
          (candidate) => candidate.kind === 'plan'
            && candidate.status === 'running'
            && candidate.planId === id,
        );
        if (!planTask) return current;
        return updateTask(current, planTask.id, {
          phase: progress.phase,
          message: progress.message,
          progress: null,
          step: progress.step,
          totalSteps: progress.totalSteps,
          bytesDownloaded: progress.bytesDownloaded,
          bytesTotal: progress.bytesTotal,
        });
      });
    });

    const unlistenPack = listen<PackInstallProgressEvent>('pack-install-progress', (event) => {
      const progress = event.payload;
      setTaskMap((current) => {
        const activeTasks = Object.values(current)
          .filter((task) => task.kind === 'modrinth-pack' && task.status === 'running')
          .sort((left, right) => right.id.localeCompare(left.id));
        const candidate = activeTasks.find((task) => task.operationId === progress.operationId)
          ?? activeTasks.find((task) => !task.operationId)
          ?? (activeTasks.length === 1 ? activeTasks[0] : undefined);
        if (!candidate) return current;
        return updateTask(current, candidate.id, {
          operationId: progress.operationId,
          phase: progress.phase,
          message: progress.message,
          progress: progress.progress ?? null,
          step: progress.step ?? 0,
          totalSteps: progress.totalSteps ?? 0,
          bytesDownloaded: progress.bytesDownloaded ?? 0,
          bytesTotal: progress.bytesTotal ?? 0,
        });
      });
    });

    return () => {
      void unlistenPlan.then((remove) => remove());
      void unlistenPack.then((remove) => remove());
    };
  }, []);

  const completeTask = (id: string, outcome: InstallOutcome) => {
    const success = outcome.type === 'success';
    setTaskMap((current) => updateTask(current, id, {
      status: success ? 'completed' : 'failed',
      phase: success ? 'done' : outcome.type,
      message: success ? 'Pack installed successfully.' : 'Pack installation did not complete.',
      progress: success ? 1 : null,
      error: success ? null : outcome.type === 'failed' ? outcome.error : 'Installation was cancelled.',
    }));
    setRevision((value) => value + 1);
    window.setTimeout(() => {
      setTaskMap((current) => {
        if (!current[id] || current[id].status === 'running') return current;
        const next = { ...current };
        delete next[id];
        return next;
      });
    }, 8000);
  };

  const failTask = (id: string, error: string) => {
    setTaskMap((current) => updateTask(current, id, {
      status: 'failed',
      phase: 'failed',
      message: 'Pack installation failed.',
      error,
    }));
    setRevision((value) => value + 1);
    window.setTimeout(() => {
      setTaskMap((current) => {
        if (!current[id] || current[id].status === 'running') return current;
        const next = { ...current };
        delete next[id];
        return next;
      });
    }, 8000);
  };

  const completeImportedTask = (id: string, instanceId: string, label: string) => {
    setTaskMap((current) => updateTask(current, id, {
      instanceId,
      instanceName: label,
      status: 'completed',
      phase: 'done',
      message: 'Pack installed successfully.',
      progress: 1,
    }));
    setRevision((value) => value + 1);
    window.setTimeout(() => {
      setTaskMap((current) => {
        if (!current[id] || current[id].status === 'running') return current;
        const next = { ...current };
        delete next[id];
        return next;
      });
    }, 8000);
  };

  const startPlan = (plan: ResolvedInstallPlan, label: string, instanceName?: string) => {
    const id = `pack-plan-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const task = {
      ...initialTask(id, label, 'plan', plan.intent.targetInstance, instanceName ?? plan.intent.targetInstance),
      planId: plan.fingerprint,
    };
    setTaskMap((current) => ({ ...current, [id]: task }));
    void applyInstallPlan(plan)
      .then((outcome) => completeTask(id, outcome))
      .catch((cause) => failTask(id, formatError(cause)));
  };

  const startModrinthPack = (downloadUrl: string, label: string) => {
    const id = `modrinth-pack-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    setTaskMap((current) => ({
      ...current,
      [id]: initialTask(id, label, 'modrinth-pack', null, label),
    }));
    void importModrinthPackByUrl(downloadUrl)
      .then((instanceId) => completeImportedTask(id, instanceId, label))
      .catch((cause) => failTask(id, formatError(cause)));
  };

  const startPackFile = (sourcePath: string, label: string) => {
    const id = `pack-file-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    setTaskMap((current) => ({
      ...current,
      [id]: initialTask(id, label, 'pack-file', null, label),
    }));
    void importInstancePack(sourcePath)
      .then((instanceId) => completeImportedTask(id, instanceId, label))
      .catch((cause) => failTask(id, formatError(cause)));
  };

  const value = useMemo<PackInstallContextValue>(() => {
    const tasks = Object.values(taskMap).sort((left, right) => left.id.localeCompare(right.id));
    return {
      revision,
      tasks,
      getTaskForInstance: (instanceId) =>
        tasks.find((task) => task.instanceId === instanceId) ?? null,
      startPlan,
      startModrinthPack,
      startPackFile,
    };
  }, [revision, taskMap]);

  return (
    <PackInstallContext.Provider value={value}>
      {children}
      <PackInstallIndicator tasks={value.tasks} />
    </PackInstallContext.Provider>
  );
}

export function usePackInstall() {
  const context = useContext(PackInstallContext);
  if (!context) throw new Error('usePackInstall must be used inside PackInstallProvider');
  return context;
}

export function PackInstallProgressBar({ task, compact = false }: { task: PackInstallTask; compact?: boolean }) {
  const percent = displayPercent(task);
  const detail = task.bytesTotal > 0
    ? `${formatBytes(task.bytesDownloaded)} / ${formatBytes(task.bytesTotal)}`
    : task.totalSteps > 0
      ? `File ${Math.min(task.step, task.totalSteps)} of ${task.totalSteps}`
      : null;

  return (
    <div className={compact ? 'mt-3 rounded-lg border border-primary/30 bg-primary/5 p-3 space-y-2' : 'space-y-2'} aria-live="polite">
      <div className="flex items-center justify-between gap-2">
        <p className="min-w-0 truncate text-sm font-medium">
          {task.status === 'completed' ? 'Pack installed' : task.status === 'failed' ? 'Pack installation failed' : phaseLabel(task.phase)}
        </p>
        {percent !== null && <span className="shrink-0 text-xs text-muted-foreground">{percent}%</span>}
      </div>
      <div
        className="h-2 overflow-hidden rounded-full bg-muted"
        role="progressbar"
        aria-label={`${task.label} progress`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
      >
        <div
          className={percent === null ? 'h-full w-1/3 animate-pulse rounded-full bg-primary' : 'h-full rounded-full bg-primary transition-all duration-300'}
          style={percent === null ? undefined : { width: `${percent}%` }}
        />
      </div>
      <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
        <span className="min-w-0 truncate" title={task.message}>{task.message}</span>
        {detail && <span className="shrink-0">{detail}</span>}
      </div>
      {task.error && <p className="text-xs text-destructive">{task.error}</p>}
    </div>
  );
}

function PackInstallIndicator({ tasks }: { tasks: PackInstallTask[] }) {
  if (tasks.length === 0) return null;
  return (
    <aside className="pointer-events-none fixed bottom-4 right-4 z-[60] w-[min(24rem,calc(100vw-2rem))] space-y-2">
      {tasks.map((task) => (
        <div key={task.id} className="pointer-events-auto rounded-xl border border-border bg-card/95 p-4 shadow-2xl backdrop-blur">
          <div className="mb-2 flex items-center justify-between gap-3">
            <div className="min-w-0">
              <p className="truncate text-sm font-semibold">{task.label}</p>
              <p className="truncate text-xs text-muted-foreground">
                {task.instanceName}
              </p>
            </div>
            {task.status === 'running' && <div className="h-4 w-4 shrink-0 animate-spin rounded-full border-2 border-primary border-t-transparent" />}
          </div>
          <PackInstallProgressBar task={task} />
        </div>
      ))}
    </aside>
  );
}
