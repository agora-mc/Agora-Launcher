/**
 * Plain-English words and small pure helpers for the Tools tab. Kept apart from the components so
 * the wording that matters (what a swap costs, why a run was discarded) is tested directly.
 */

import type { CaptureChoice, OutputStatus, RunReport, SwapCosts, ToolRow } from './api';
import { isGamesError } from '../shared/types';

/** How many paths a list shows before it says "and N more" (the core reports the same 20). */
export const SHOWN_PATHS = 20;

/** The words a failed call shows. A `GamesError` carries its own message for people. */
export function messageOf(error: unknown): string {
  if (isGamesError(error)) return error.message;
  if (error instanceof Error) return error.message;
  return 'Something went wrong. Try again.';
}

/** The capture choices, in the order the screen lists them. `swap` is listed last and never the default. */
export const CAPTURE_CHOICES: readonly { value: CaptureChoice; label: string; help: string }[] = [
  {
    value: 'auto',
    label: 'Let Agora choose',
    help: 'Uses the way this instance is set up. Agora asks before it uses anything slower.',
  },
  {
    value: 'vfs',
    label: 'Virtual file system',
    help: 'Runs the tool through the instance’s file layer. Refused if that layer cannot start.',
  },
  {
    value: 'links',
    label: 'Link capture',
    help: 'Runs the tool in the game folder and records what it wrote there.',
  },
  {
    value: 'swap',
    label: 'Full-copy swap',
    help: 'Copies all of the game’s Data folder for the run. Shows its cost first.',
  },
];

/** The words for an output status, as a badge. */
export function statusLabel(status: OutputStatus | null): string {
  switch (status) {
    case 'current':
      return 'Up to date';
    case 'stale':
      return 'Out of date';
    case 'unknown':
      return 'Rebuild to be sure';
    case 'conflict':
      return 'Conflicts with another tool';
    case null:
      return 'Not built yet';
  }
}

/** One sentence on what the status means for the player, under the badge. */
export function statusExplanation(row: ToolRow): string {
  switch (row.status) {
    case 'current':
      return 'Its output matches the mods and settings this instance has now.';
    case 'stale':
      return 'A mod, the load order or a setting changed since it was built. Rebuild it so the game reads current output.';
    case 'unknown':
      return 'This output was imported, so Agora cannot tell whether it still matches. Rebuild it to be sure.';
    case 'conflict':
      return 'Another tool here has output turned on, and both write the same behaviour files.';
    case null:
      return 'Nothing has been built for this instance yet.';
  }
}

/** The bytes, in words a person reads. Matches the core's wording, so the two never disagree. */
export function humanBytes(bytes: number): string {
  const GB = 1024 * 1024 * 1024;
  const MB = 1024 * 1024;
  if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
  if (bytes >= MB) return `${Math.round(bytes / MB)} MB`;
  return `${bytes} bytes`;
}

/** The first `limit` paths, and how many more there are. */
export function shownPaths(paths: readonly string[], limit = SHOWN_PATHS): { shown: string[]; more: number } {
  return { shown: paths.slice(0, limit), more: Math.max(0, paths.length - limit) };
}

/** The lines that say what a swap costs and what it does, before anyone agrees to it. */
export function swapCostLines(toolName: string, costs: SwapCosts): string[] {
  const lines = [
    `${toolName} works on the game’s Data folder, so it runs on a copy of it, swapped in for the length of the run.`,
    costs.freeBytes === null
      ? `Disk space: the copy needs ${humanBytes(costs.copyBytes)} on the drive of ${costs.copyFolder}. The free space there could not be read.`
      : `Disk space: the copy needs ${humanBytes(costs.copyBytes)} on the drive of ${costs.copyFolder}, and ${humanBytes(costs.freeBytes)} is free there.`,
    `The game’s Data is ${humanBytes(costs.dataBytes)} in ${costs.dataFiles} file(s).`,
  ];
  if (costs.sameVolume && costs.keptCopyExists) {
    lines.push(`A kept copy from an earlier run is there, so this run copies only what changed: ${humanBytes(costs.copyBytes)} in ${costs.copyFiles} file(s).`);
  } else if (costs.sameVolume) {
    lines.push(`The copy is kept between runs on this drive. The first run copies everything (${humanBytes(costs.copyBytes)} in ${costs.copyFiles} file(s)); later runs copy only what changed.`);
  } else {
    lines.push(`The copy would be on another drive from the game, so every run copies all of Data in (${humanBytes(costs.copyBytes)} in ${costs.copyFiles} file(s)) and the changes back out. The copy is deleted after the run.`);
  }
  lines.push('The game’s real Data folder is renamed for the length of the run. Steam must not update the game, and the game must not be started meanwhile.');
  if (costs.steamRunning) {
    lines.push('Warning: Steam is running. Close it first, or it may update the game while Data is renamed.');
  }
  return lines;
}

/** The name of the tier that captured a run, for a person. */
export function captureName(capture: RunReport['capture']): string {
  switch (capture) {
    case 'vfs':
      return 'the virtual file system';
    case 'links':
      return 'link capture';
    case 'swap':
      return 'the full-copy swap';
  }
}

/** The headline of a finished run: promoted, or discarded and why. */
export function runHeadline(outcome: RunReport): string {
  if (outcome.cancelled) {
    return `${outcome.name} was stopped before it finished. Nothing changed.`;
  }
  if (outcome.promoted) {
    return `${outcome.name} finished, and its output is now generation ${outcome.current ?? '?'}. The game reads it on its next launch.`;
  }
  return `${outcome.name} did not produce usable output, so nothing changed. The current output stays in effect.`;
}

/** The lines under a finished run's headline: why it was discarded, where it was kept, what else it did. */
export function runDetailLines(outcome: RunReport): string[] {
  const lines: string[] = [];
  if (!outcome.promoted && outcome.failure) {
    lines.push(`Why: ${outcome.failure}`);
  }
  if (!outcome.promoted && outcome.failedFolder) {
    lines.push(`The files it wrote are kept for diagnostics in ${outcome.failedFolder}.`);
  }
  if (outcome.promoted) {
    lines.push(`It wrote ${outcome.written.length} file(s)${outcome.deleted.length > 0 ? ` and removed ${outcome.deleted.length}` : ''}.`);
  }
  lines.push(`Captured by ${captureName(outcome.capture)}.`);
  if (outcome.captureReason) {
    lines.push(`It runs this way because: ${outcome.captureReason}`);
  }
  if (outcome.unchangedDropped > 0) {
    lines.push(`${outcome.unchangedDropped} file(s) the tool opened but did not change were left out of the output.`);
  }
  if (outcome.copiedLayer) {
    lines.push(`${outcome.copiedLayer}’s own files were copied for this run, so the tool could rewrite them. The mod itself is unchanged.`);
  }
  const changes = outcome.installChanges;
  const outside = changes.created.length + changes.changed.length + changes.deleted.length;
  if (outside > 0) {
    lines.push(`The tool also wrote ${outside} path(s) in the game folder, outside what Agora captured. They are not part of the output.`);
  }
  if (changes.unavailable) {
    lines.push(`The game folder could not be watched (${changes.unavailable}), so its writes are not known.`);
  }
  if (changes.mayBeIncomplete) {
    lines.push('Changes may have been missed: the watch of the game folder lost events.');
  }
  return lines;
}

/** The words for a tool's conflict warning, naming the other tool. */
export function conflictLine(row: ToolRow, others: readonly ToolRow[]): string {
  const names = others.filter((other) => other.conflicts && other.toolId !== row.toolId).map((other) => other.name);
  const list = names.length > 0 ? names.join(', ') : 'another tool';
  return `${row.name} and ${list} both have output here, and both write the same behaviour files. Remove the one you do not want to use.`;
}
