/**
 * The words the bases screens show, kept apart from the components so they can be tested. Every
 * sentence here is for a modder who is not a programmer: what happened, why it matters, what to do.
 */

import { formatBytes } from '@/components/installed-content/contentTableState';
import type { BaseFileProblem, BaseMode, BaseSummary, BaseUser, BuildProgressEvent, VerifyReply } from './api';

export function modeLabel(mode: BaseMode): string {
  return mode === 'linked' ? 'Linked' : 'Copied';
}

/** What a base of this mode is, and what it costs. Shown where the user chooses a mode. */
export const MODE_EXPLANATIONS: Record<BaseMode, { title: string; body: string }> = {
  linked: {
    title: 'Linked (recommended)',
    body: 'The big game archives are hard links to your store install, so they take almost no extra disk space. '
      + 'Everything else is copied. It needs the base on the same drive as the game. If the store changes an archive in place, '
      + 'Agora reports the change instead of hiding it.',
  },
  copied: {
    title: 'Copied',
    body: 'Every file is copied. It takes about as much disk space as the game itself, but it works on any drive '
      + 'and nothing the store does to the game can change it.',
  },
};

/**
 * The cost of a Copied base before it is built, from the install's size. Says so plainly when the
 * size is not known rather than showing 0.
 */
export function copiedCostText(installBytes: number | null | undefined): string {
  if (installBytes == null) {
    return 'Agora could not measure this install, so it cannot say how much space a Copied base takes.';
  }
  return `A Copied base of this install takes about ${formatBytes(installBytes)} of disk space.`;
}

export function describeUsers(usedBy: BaseUser[]): string {
  if (usedBy.length === 0) return 'No instance uses this base, so it can be removed.';
  // A blank name still names the user: the instance id is shown instead of an empty gap.
  const names = usedBy.map((user) => user.name.trim() || user.instanceId).join(', ');
  return `Used by ${names}. Remove or move those instances before removing this base.`;
}

/** Remove is refused while any instance uses the base; the button says so before the click. */
export function canRemove(base: { usedBy: BaseUser[] }): boolean {
  return base.usedBy.length === 0;
}

export const DAMAGED_TITLE = 'Damaged base record';

/** What a damaged record means for the user, in plain words. Shown on its card. */
export const DAMAGED_EXPLANATION = 'Agora cannot read this base\'s record file, so it does not know which game or version '
  + 'the base is for, or where its folder is. Agora cannot check the base. Removing it deletes the record, and you can build '
  + 'the base again afterwards if you still need it.';

/** What removing a damaged record deletes. Only the folder inside Agora's bases folder can be found. */
export const DAMAGED_REMOVE_BODY = 'Its record file is deleted, and its folder too if it is inside the Agora bases folder. '
  + 'A base built on another drive keeps its folder there: look for an AgoraBases folder on that drive and delete it yourself. '
  + 'The game in your store is not touched.';

export function diskText(base: BaseSummary): string {
  if (base.mode === 'copied') {
    return `Holds ${formatBytes(base.copiedBytes)} of its own.`;
  }
  return `Holds ${formatBytes(base.copiedBytes)} of its own and shares ${formatBytes(base.linkedBytes)} with the store install, which takes no extra space.`;
}

/** What a problem with one file means, in plain words. */
export function describeProblem(problem: BaseFileProblem): string {
  switch (problem.kind) {
    case 'missing':
      return 'The file is gone from the base.';
    case 'size_changed':
      return `The file changed size (it should be ${formatBytes(problem.expectedBytes)}, it is ${formatBytes(problem.actualBytes)}).`;
    case 'content_changed':
      return 'The file no longer matches the hash Agora recorded when it built the base.';
    case 'unexpected':
      return 'The file is not part of the base. Something was written into it.';
  }
}

/** The line a problem gets when the file is also a link to the store install. */
export function linkedWarning(problem: BaseFileProblem): string | null {
  return problem.linkedToStore
    ? 'This file is a hard link, so the change also reached the store install.'
    : null;
}

export function verifySummary(reply: VerifyReply): string {
  const depth = reply.depth === 'full' ? 'Every file was hashed again' : 'Sizes and times were checked';
  if (reply.clean) {
    return `Clean. ${depth}: ${reply.checked} files checked, ${reply.hashed} hashed.`;
  }
  const count = reply.problems.length;
  return `Damaged: ${count} ${count === 1 ? 'file needs' : 'files need'} attention. ${depth}: ${reply.checked} files checked.`;
}

/** Game writes are kept in each instance; they are listed so the user knows they are not damage. */
export function gameWritesNote(gameWrites: string[]): string | null {
  if (gameWrites.length === 0) return null;
  return `The game wrote to ${gameWrites.length === 1 ? 'this file' : 'these files'}: ${gameWrites.join(', ')}. `
    + 'Agora keeps those writes in each instance, so they are not damage.';
}

export function progressPercent(progress: BuildProgressEvent): number {
  if (progress.bytesTotal <= 0) return progress.filesTotal > 0 ? Math.round((progress.filesDone / progress.filesTotal) * 100) : 0;
  return Math.min(100, Math.round((progress.bytesHashed / progress.bytesTotal) * 100));
}

export function progressText(progress: BuildProgressEvent): string {
  return `Hashed ${progress.filesDone} of ${progress.filesTotal} files, ${formatBytes(progress.bytesHashed)} of ${formatBytes(progress.bytesTotal)}.`;
}

export const LINK_UNAVAILABLE_TITLE = 'Hard links are not available for this install';

export function copiedNeededText(bytes: number): string {
  return `A Copied base would take ${formatBytes(bytes)} on this drive.`;
}
