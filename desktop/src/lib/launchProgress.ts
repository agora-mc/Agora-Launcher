/**
 * Launch-stage model for the `launch-progress` events the backend emits while
 * a launch is being prepared and while the game is starting up.
 *
 * "Ready" comes from the game's own log output (see `launch_stage.rs`), so it
 * can be a few seconds before the title screen, and versions that do not log
 * the signal stay in "Running — loading". That is reported as-is, never faked.
 */
export type LaunchStageId =
  | 'checking'
  | 'java'
  | 'preparing'
  | 'files'
  | 'snapshot'
  | 'starting'
  | 'loading'
  | 'ready'
  | 'handoff';

export interface LaunchFilesProgress {
  kind: 'client' | 'libraries' | 'assets' | string;
  done: number;
  total: number;
}

export interface LaunchProgressInfo {
  stage: LaunchStageId;
  /** Human label for the current stage. */
  label: string;
  /** Epoch ms when this launch began, for the elapsed-time readout. */
  startedAt: number;
  files: LaunchFilesProgress | null;
  /** 0-100 for stages that report a percentage rather than file counts (Java). */
  percent?: number | null;
  /** Status text to show beside the label, e.g. "Downloading Java 21: 40 of 52 MB". */
  detail?: string | null;
}

export interface LaunchProgressEventPayload {
  instance_id: string;
  phase: string;
  message?: string;
  files?: LaunchFilesProgress;
  percent?: number | null;
}

export const STARTING_PROGRESS = (now: number): LaunchProgressInfo => ({
  stage: 'starting',
  label: 'Starting',
  startedAt: now,
  files: null,
});

const KIND_LABEL: Record<string, string> = {
  client: 'game jar',
  libraries: 'libraries',
  assets: 'assets',
};

/** Fold one backend event into the running progress; null means "ignore it". */
export function applyLaunchProgressEvent(
  previous: LaunchProgressInfo | null,
  event: LaunchProgressEventPayload,
  now: number,
): LaunchProgressInfo | null {
  // "<phase>-complete" events only carry timings.
  if (event.phase.endsWith('-complete')) return previous;
  const startedAt = event.phase === 'checking-health' ? now : previous?.startedAt ?? now;
  const base = { startedAt, files: null };
  switch (event.phase) {
    case 'checking-health':
      return { ...base, stage: 'checking', label: 'Checking the instance' };
    case 'resolving':
      return { ...base, stage: 'preparing', label: event.message || 'Preparing Java and the mod loader' };
    case 'provisioning-java': {
      const reported = typeof event.percent === 'number';
      return {
        ...base,
        stage: 'java',
        label: 'Downloading Java',
        percent: reported ? Math.max(0, Math.min(100, Math.round(event.percent as number))) : null,
        detail: reported ? event.message ?? null : null,
      };
    }
    case 'materializing':
      return {
        ...base,
        stage: 'files',
        label: 'Downloading and verifying game files',
        files: event.files ?? previous?.files ?? null,
      };
    case 'snapshot':
      return { ...base, stage: 'snapshot', label: 'Saving a recovery snapshot' };
    case 'launching':
      return { ...base, stage: 'starting', label: 'Starting Minecraft' };
    case 'running':
      return { ...base, stage: 'loading', label: 'Running — loading' };
    case 'ready':
      return { ...base, stage: 'ready', label: 'Ready' };
    case 'handoff':
      return { ...base, stage: 'handoff', label: 'Handing off to the Minecraft Launcher' };
    default:
      return previous;
  }
}

/** "libraries 12 / 80" style detail for the files stage, or null. */
export function filesDetail(files: LaunchFilesProgress | null): string | null {
  if (!files || files.total <= 0) return null;
  return `${KIND_LABEL[files.kind] ?? files.kind} ${files.done} / ${files.total}`;
}

/** 0-100 for a determinate bar, or null when counts are not known. */
export function filesPercent(files: LaunchFilesProgress | null): number | null {
  if (!files || files.total <= 0) return null;
  return Math.max(0, Math.min(100, Math.round((files.done / files.total) * 100)));
}

export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return minutes > 0 ? `${minutes}m ${String(seconds).padStart(2, '0')}s` : `${seconds}s`;
}
