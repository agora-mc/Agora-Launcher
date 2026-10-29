/**
 * Plain-English words for the instances pages: what each deployment mode means, what a base is,
 * and how a time reads. Kept apart from the components so the copy can be tested on its own.
 */

import type { BaseMode, CheckFinding, DeploymentChoice, DeploymentMode, InstanceCheck } from './api';

export interface DeploymentOption {
  id: DeploymentChoice;
  label: string;
  /** What the user gets from this mode, and what it costs, in one or two sentences. */
  explain: string;
}

/** The four choices, in the order the page lists them. Automatic is first: it is the default. */
export const DEPLOYMENT_OPTIONS: readonly DeploymentOption[] = [
  {
    id: 'auto',
    label: 'Automatic',
    explain: 'Agora uses the game\'s default. If that cannot run, Agora says so and asks before it steps down.',
  },
  {
    id: 'virtual',
    label: 'Virtual file system',
    explain: 'Every write the game makes lands in this instance\'s own folder, so the game install is never changed. Needs Agora\'s virtual file system driver.',
  },
  {
    id: 'links',
    label: 'Linked files',
    explain: 'Mod files are hard links into the game folder, so nothing is copied. A mod that edits its own files in place may be refused.',
  },
  {
    id: 'copies',
    label: 'Copied files',
    explain: 'Every mod file is a real copy in the game folder. Every file can be written, but copies take more disk space and longer to switch between instances.',
  },
];

const DEPLOYMENT_LABELS: Record<DeploymentMode, string> = {
  virtual: 'Virtual file system',
  links: 'Linked files',
  copies: 'Copied files',
};

/** The name a deployment choice shows as. `null` is the automatic choice. */
export function deploymentLabel(mode: DeploymentMode | null): string {
  return mode === null ? 'Automatic' : DEPLOYMENT_LABELS[mode];
}

/** What running from `next` gives up, for the question asked when the virtual file system will not start. */
export function fallbackTradeoffs(next: DeploymentMode): string[] {
  switch (next) {
    case 'links':
      return [
        'Linked files cannot catch writes: a mod that edits its own files in place may be refused.',
        'Small config files are copies, so most mods still work.',
      ];
    case 'copies':
      return [
        'Copied files take more disk space and longer to switch between instances.',
        'Every file can be written, but only in this instance\'s own copy.',
      ];
    case 'virtual':
      return [];
  }
}

export function baseModeCopy(mode: BaseMode): { label: string; explain: string } {
  return mode === 'linked'
    ? {
        label: 'Linked base',
        explain: 'Files are shared with the game install, so the base takes almost no extra disk space. A change to a linked file also changes the install.',
      }
    : {
        label: 'Copied base',
        explain: 'The base keeps its own copy of the game files. It takes up about the size of the game again, but changes to it never reach the install.',
      };
}

/** A check's one-line summary for its header: what is blocking, what to review, or that all is well. */
export function checkHeadline(check: InstanceCheck | null): string {
  if (check === null) return 'Checking this instance…';
  if (!check.checked) return 'This instance runs from its install folder, so there is no framework check.';
  switch (check.status) {
    case 'ok':
      return 'Everything Agora checks is in order.';
    case 'warnings':
      return warningsText(check.findings.length);
    case 'findings':
      return blockingText(check.findings.filter((finding) => finding.blocks).length);
  }
}

export function blockingText(count: number): string {
  return `${count} ${count === 1 ? 'problem stops' : 'problems stop'} the game from launching`;
}

export function warningsText(count: number): string {
  return `${count} ${count === 1 ? 'thing' : 'things'} to review`;
}

/** Whether the framework check passed, failed, or was not run. Null when no check has answered. */
export function frameworksMatch(check: InstanceCheck | null): boolean | null {
  if (check === null || !check.checked) return null;
  return !check.findings.some((finding) => finding.kind === 'framework' && finding.blocks);
}

/** The findings that stop a launch, then the ones to review, in the order the page shows them. */
export function orderFindings(findings: readonly CheckFinding[]): CheckFinding[] {
  return [...findings].sort((a, b) => Number(b.blocks) - Number(a.blocks));
}

/**
 * A time from the backend. SQLite writes `2026-10-01 10:00:00` in UTC without a zone, so it is read
 * as UTC. Anything that does not parse is shown as it came, never as a made-up date.
 */
export function formatWhen(value: string | null): string {
  if (value === null || value.trim() === '') return 'Never';
  const parsed = new Date(`${value.trim().replace(' ', 'T')}Z`);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString();
}

/** A file count and a size, for the progress of a base build. */
export function progressText(done: number, total: number): string {
  if (total <= 0) return 'Starting…';
  return `${done.toLocaleString()} of ${total.toLocaleString()} files`;
}

/** Elapsed seconds as the page reads them: `42 s`, then `2 min 05 s`. */
export function elapsedText(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  if (whole < 60) return `${whole} s`;
  return `${Math.floor(whole / 60)} min ${String(whole % 60).padStart(2, '0')} s`;
}
