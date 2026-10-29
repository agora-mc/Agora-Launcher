/**
 * IPC wrappers for the bases slice (pinned copies of a game that instances are built from,
 * MASTER_SPEC §26.4).
 *
 * Every reply is checked against the shape the backend documents before it reaches a caller. A
 * reply that is not that shape rejects: a caller must never read a missing answer as "no bases",
 * "clean", or "removed".
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

/** Emitted while a base is being built. The payload is `BuildProgressEvent`. */
export const BUILD_PROGRESS_EVENT = 'games-base-progress';

export type BaseMode = 'linked' | 'copied';

export interface BaseUser {
  instanceId: string;
  name: string;
}

export interface BaseSummary {
  baseId: string;
  gameId: string;
  store: string;
  version: string;
  build: string | null;
  mode: BaseMode;
  location: string;
  /** False when the base folder is gone. Instances on it cannot launch until it is rebuilt. */
  present: boolean;
  fileCount: number;
  linkedFileCount: number;
  linkedBytes: number;
  copiedBytes: number;
  skippedCount: number;
  createdUnixMs: number;
  /** The instances pinned to this base. Removing the base is refused while this is not empty. */
  usedBy: BaseUser[];
}

/**
 * A base record the backend cannot read. Its game, version and folder are unknown, so it cannot be
 * checked; it can be listed, and removed once no instance uses it.
 */
export interface DamagedBaseSummary {
  baseId: string;
  /** The record's file, so the user can see what Agora could not read. */
  manifestPath: string;
  /** Why the record cannot be used: the file and the cause. */
  error: string;
  /** The instances pinned to this base. Removing the base is refused while this is not empty. */
  usedBy: BaseUser[];
}

export interface BasesList {
  bases: BaseSummary[];
  /** Records that cannot be read. Listed for every game, because a damaged record does not say which game it is for. */
  damaged: DamagedBaseSummary[];
}

export type BuildReply =
  | { status: 'built'; baseId: string; mode: BaseMode; location: string; fileCount: number; linkedBytes: number; copiedBytes: number }
  | { status: 'existing'; baseId: string; mode: BaseMode; location: string; fileCount: number }
  | { status: 'link_unavailable'; reason: string; copiedBytesNeeded: number };

export interface BuildProgressEvent {
  installId: string;
  filesDone: number;
  filesTotal: number;
  bytesHashed: number;
  bytesTotal: number;
}

export type ProblemKindName = 'missing' | 'size_changed' | 'content_changed' | 'unexpected';

export interface BaseFileProblem {
  path: string;
  kind: ProblemKindName;
  expectedBytes: number | null;
  actualBytes: number | null;
  /** The file is a hardlink to the store install, so the change reached the store install too. */
  linkedToStore: boolean;
}

export interface VerifyReply {
  baseId: string;
  depth: 'quick' | 'full';
  checked: number;
  hashed: number;
  clean: boolean;
  /** Files the game wrote into the base. Those are kept in each instance, not a damage. */
  gameWrites: string[];
  problems: BaseFileProblem[];
}

export interface RemoveReply {
  baseId: string;
  status: 'removed';
}

// ---------------------------------------------------------------------------
// Shape checks. Each one accepts a value only when every field the UI reads is present and typed.
// ---------------------------------------------------------------------------

type Obj = Record<string, unknown>;

const isObj = (value: unknown): value is Obj => typeof value === 'object' && value !== null;
const isStr = (value: unknown): value is string => typeof value === 'string';
const isNum = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value);
const isBool = (value: unknown): value is boolean => typeof value === 'boolean';
const isStrOrNull = (value: unknown): value is string | null => value === null || isStr(value);
const isNumOrNull = (value: unknown): value is number | null => value === null || isNum(value);
const isMode = (value: unknown): value is BaseMode => value === 'linked' || value === 'copied';

function isBaseUser(value: unknown): value is BaseUser {
  return isObj(value) && isStr(value.instanceId) && isStr(value.name);
}

function isBaseSummary(value: unknown): value is BaseSummary {
  return isObj(value)
    && isStr(value.baseId)
    && isStr(value.gameId)
    && isStr(value.store)
    && isStr(value.version)
    && isStrOrNull(value.build)
    && isMode(value.mode)
    && isStr(value.location)
    && isBool(value.present)
    && isNum(value.fileCount)
    && isNum(value.linkedFileCount)
    && isNum(value.linkedBytes)
    && isNum(value.copiedBytes)
    && isNum(value.skippedCount)
    && isNum(value.createdUnixMs)
    && Array.isArray(value.usedBy)
    && value.usedBy.every(isBaseUser);
}

function isDamagedBaseSummary(value: unknown): value is DamagedBaseSummary {
  return isObj(value)
    && isStr(value.baseId)
    && isStr(value.manifestPath)
    && isStr(value.error)
    && Array.isArray(value.usedBy)
    && value.usedBy.every(isBaseUser);
}

function isBuildReply(value: unknown): value is BuildReply {
  if (!isObj(value)) return false;
  if (value.status === 'built') {
    return isStr(value.baseId) && isMode(value.mode) && isStr(value.location)
      && isNum(value.fileCount) && isNum(value.linkedBytes) && isNum(value.copiedBytes);
  }
  if (value.status === 'existing') {
    return isStr(value.baseId) && isMode(value.mode) && isStr(value.location) && isNum(value.fileCount);
  }
  if (value.status === 'link_unavailable') {
    return isStr(value.reason) && isNum(value.copiedBytesNeeded);
  }
  return false;
}

const PROBLEM_KINDS: readonly string[] = ['missing', 'size_changed', 'content_changed', 'unexpected'];

function isBaseFileProblem(value: unknown): value is BaseFileProblem {
  return isObj(value)
    && isStr(value.path)
    && isStr(value.kind) && PROBLEM_KINDS.includes(value.kind)
    && isNumOrNull(value.expectedBytes)
    && isNumOrNull(value.actualBytes)
    && isBool(value.linkedToStore);
}

function isVerifyReply(value: unknown): value is VerifyReply {
  return isObj(value)
    && isStr(value.baseId)
    && (value.depth === 'quick' || value.depth === 'full')
    && isNum(value.checked)
    && isNum(value.hashed)
    && isBool(value.clean)
    && Array.isArray(value.gameWrites) && value.gameWrites.every(isStr)
    && Array.isArray(value.problems) && value.problems.every(isBaseFileProblem);
}

function isRemoveReply(value: unknown): value is RemoveReply {
  return isObj(value) && isStr(value.baseId) && value.status === 'removed';
}

function reject(command: string): never {
  throw new Error(`${command} did not return the documented shape.`);
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/** The bases built for one game, and the records that cannot be read. Rejects if the reply is not that shape. */
export async function listBases(gameId: string): Promise<BasesList> {
  const reply: unknown = await invoke<unknown>('games_base_list', { gameId });
  if (
    !isObj(reply)
    || !Array.isArray(reply.bases) || !reply.bases.every(isBaseSummary)
    || !Array.isArray(reply.damaged) || !reply.damaged.every(isDamagedBaseSummary)
  ) {
    reject('games_base_list');
  }
  return reply as unknown as BasesList;
}

/** Build a base from a discovered install. The user picks the mode; nothing is decided for them. */
export async function buildBase(installId: string, mode: BaseMode): Promise<BuildReply> {
  const reply: unknown = await invoke<unknown>('games_base_build', { installId, mode, includeExcluded: false });
  if (!isBuildReply(reply)) reject('games_base_build');
  return reply as BuildReply;
}

/** Check a base against its manifest. `full` hashes every file again; otherwise sizes and times only. */
export async function verifyBase(baseId: string, full: boolean): Promise<VerifyReply> {
  const reply: unknown = await invoke<unknown>('games_base_verify', { baseId, full });
  if (!isVerifyReply(reply)) reject('games_base_verify');
  return reply as VerifyReply;
}

/** Delete a base no instance uses. The backend refuses, and names the instances, while any uses it. */
export async function removeBase(baseId: string): Promise<RemoveReply> {
  const reply: unknown = await invoke<unknown>('games_base_remove', { baseId });
  if (!isRemoveReply(reply)) reject('games_base_remove');
  return reply as RemoveReply;
}

/** Progress for builds, as the backend hashes files. Resolves to the function that stops listening. */
export function listenBuildProgress(onProgress: (event: BuildProgressEvent) => void): Promise<UnlistenFn> {
  return listen<BuildProgressEvent>(BUILD_PROGRESS_EVENT, (event) => {
    onProgress(event.payload);
  });
}
