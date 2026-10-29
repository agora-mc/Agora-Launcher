/**
 * IPC wrappers for the instances slice (the game instance list, an instance's page, its
 * checks and launch). Every reply is checked against the shape the backend documents, so a missing
 * answer is an error here, never "no instances".
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { isGamesError } from '../shared/types';

export type DeploymentMode = 'virtual' | 'links' | 'copies';
/** `auto` is the game's default, with any step down announced. A reply reads it as `null`. */
export type DeploymentChoice = DeploymentMode | 'auto';
export type BaseMode = 'linked' | 'copied';

export interface InstanceCard {
  instanceId: string;
  gameId: string;
  name: string;
  store: string | null;
  version: string | null;
  pinned: boolean;
  deployment: DeploymentMode | null;
  lastLaunchedAt: string | null;
  running: boolean;
}

export interface InstanceList {
  instances: InstanceCard[];
}

export interface InstanceDetail {
  instanceId: string;
  gameId: string;
  gameName: string;
  name: string;
  pinned: boolean;
  store: string | null;
  version: string | null;
  build: string | null;
  baseId: string | null;
  installId: string | null;
  unpinnedReason: string | null;
  deployment: DeploymentMode | null;
  saves: 'shared' | 'own';
  createdAt: string;
  lastLaunchedAt: string | null;
  running: boolean;
}

export type FindingKind = 'framework' | 'load_order' | 'tool' | 'base';

export interface CheckFinding {
  kind: FindingKind;
  /** True when this stops a launch unless the user launches anyway. */
  blocks: boolean;
  title: string;
  detail: string;
  why: string;
  repair: string | null;
}

export interface InstanceCheck {
  instanceId: string;
  checked: boolean;
  status: 'ok' | 'warnings' | 'findings';
  findings: CheckFinding[];
}

export type LaunchStatus = 'launched' | 'refused' | 'vfs_unavailable';

export interface LaunchReply {
  status: LaunchStatus;
  pid: number | null;
  deployment: DeploymentMode | null;
  notice: string | null;
  findings: CheckFinding[];
  reason: string | null;
  next: DeploymentMode | null;
}

export interface CreatedInstance {
  instanceId: string;
  name: string;
  base: 'pinned' | 'unpinned';
  reusedBase: boolean;
  baseId: string | null;
  unpinnedReason: string | null;
}

export interface DeletedInstance {
  instanceId: string;
  orphanedBase: string | null;
}

export interface InstanceDeployment {
  instanceId: string;
  deployment: DeploymentMode | null;
}

export interface CreateProgress {
  filesDone: number;
  filesTotal: number;
  bytesHashed: number;
  bytesTotal: number;
}

export interface InstanceExited {
  instanceId: string;
  restoreError: string | null;
}

const CREATE_PROGRESS_EVENT = 'games-instance-create-progress';
const INSTANCE_EXITED_EVENT = 'games-instance-exited';

const DEPLOYMENTS: readonly string[] = ['virtual', 'links', 'copies'];
const LAUNCH_STATUSES: readonly string[] = ['launched', 'refused', 'vfs_unavailable'];
const FINDING_KINDS: readonly string[] = ['framework', 'load_order', 'tool', 'base'];
const CHECK_STATUSES: readonly string[] = ['ok', 'warnings', 'findings'];

/** The sentence a page shows for a rejected command. A rejection that is not ours is said plainly. */
export function gamesErrorMessage(error: unknown): string {
  if (isGamesError(error)) return error.message;
  if (error instanceof Error) return error.message;
  return 'Something went wrong, and Agora did not say what.';
}

/** The code of a rejected command, or null when the rejection is not one of ours. */
export function gamesErrorCode(error: unknown): string | null {
  return isGamesError(error) ? error.code : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isOptionalString(value: unknown): boolean {
  return value === null || value === undefined || typeof value === 'string';
}

function isDeployment(value: unknown): value is DeploymentMode | null {
  return value === null || (typeof value === 'string' && DEPLOYMENTS.includes(value));
}

function isCard(value: unknown): value is InstanceCard {
  if (!isRecord(value)) return false;
  return typeof value.instanceId === 'string'
    && typeof value.gameId === 'string'
    && typeof value.name === 'string'
    && typeof value.pinned === 'boolean'
    && typeof value.running === 'boolean'
    && isOptionalString(value.store)
    && isOptionalString(value.version)
    && isOptionalString(value.lastLaunchedAt)
    && isDeployment(value.deployment);
}

function isFinding(value: unknown): value is CheckFinding {
  if (!isRecord(value)) return false;
  return typeof value.kind === 'string'
    && FINDING_KINDS.includes(value.kind)
    && typeof value.blocks === 'boolean'
    && typeof value.title === 'string'
    && typeof value.detail === 'string'
    && typeof value.why === 'string'
    && isOptionalString(value.repair);
}

function isLaunchReply(value: unknown): value is LaunchReply {
  if (!isRecord(value)) return false;
  return typeof value.status === 'string'
    && LAUNCH_STATUSES.includes(value.status)
    && Array.isArray(value.findings)
    && value.findings.every(isFinding)
    && (value.pid === null || typeof value.pid === 'number')
    && isDeployment(value.deployment)
    && isOptionalString(value.notice)
    && isOptionalString(value.reason)
    && isDeployment(value.next);
}

function shapeError(command: string): Error {
  return new Error(`${command} did not return what the Instances page expects.`);
}

/** The instances of one game. Rejects if the reply is not a list. */
export async function listGameInstances(gameId: string): Promise<InstanceList> {
  const reply: unknown = await invoke<unknown>('games_instance_list', { gameId });
  if (!isRecord(reply) || !Array.isArray(reply.instances) || !reply.instances.every(isCard)) {
    throw shapeError('games_instance_list');
  }
  return reply as unknown as InstanceList;
}

/** One instance, for its page's header. */
export async function getGameInstance(instanceId: string): Promise<InstanceDetail> {
  const reply: unknown = await invoke<unknown>('games_instance_get', { instanceId });
  if (
    !isRecord(reply)
    || typeof reply.instanceId !== 'string'
    || typeof reply.name !== 'string'
    || typeof reply.pinned !== 'boolean'
    || typeof reply.running !== 'boolean'
    || (reply.saves !== 'shared' && reply.saves !== 'own')
  ) {
    throw shapeError('games_instance_get');
  }
  return reply as unknown as InstanceDetail;
}

export async function createGameInstance(
  gameId: string,
  installId: string,
  name: string,
  baseMode: BaseMode,
): Promise<CreatedInstance> {
  const reply: unknown = await invoke<unknown>('games_instance_create', { gameId, installId, name, baseMode });
  if (!isRecord(reply) || typeof reply.instanceId !== 'string' || typeof reply.name !== 'string') {
    throw shapeError('games_instance_create');
  }
  return reply as unknown as CreatedInstance;
}

export async function deleteGameInstance(instanceId: string): Promise<DeletedInstance> {
  const reply: unknown = await invoke<unknown>('games_instance_delete', { instanceId });
  if (!isRecord(reply) || typeof reply.instanceId !== 'string' || !isOptionalString(reply.orphanedBase)) {
    throw shapeError('games_instance_delete');
  }
  return reply as unknown as DeletedInstance;
}

export async function checkGameInstance(instanceId: string): Promise<InstanceCheck> {
  const reply: unknown = await invoke<unknown>('games_instance_check', { instanceId });
  if (
    !isRecord(reply)
    || typeof reply.instanceId !== 'string'
    || typeof reply.checked !== 'boolean'
    || typeof reply.status !== 'string'
    || !CHECK_STATUSES.includes(reply.status)
    || !Array.isArray(reply.findings)
    || !reply.findings.every(isFinding)
  ) {
    throw shapeError('games_instance_check');
  }
  return reply as unknown as InstanceCheck;
}

export async function setGameInstanceDeployment(instanceId: string, mode: DeploymentChoice): Promise<InstanceDeployment> {
  const reply: unknown = await invoke<unknown>('games_instance_set_deployment', { instanceId, mode });
  if (!isRecord(reply) || typeof reply.instanceId !== 'string' || !isDeployment(reply.deployment)) {
    throw shapeError('games_instance_set_deployment');
  }
  return reply as unknown as InstanceDeployment;
}

/**
 * Launch an instance. `deployment` runs this launch only; leave it out to use the instance's own
 * choice. Rejects when the launch itself failed. A launch that a check or the virtual file system
 * stopped comes back as a reply whose `status` says so.
 */
export async function launchGameInstance(
  instanceId: string,
  options: { launchAnyway: boolean; deployment?: DeploymentMode },
): Promise<LaunchReply> {
  const reply: unknown = await invoke<unknown>('games_instance_launch', {
    instanceId,
    launchAnyway: options.launchAnyway,
    deployment: options.deployment ?? null,
  });
  if (!isLaunchReply(reply)) throw shapeError('games_instance_launch');
  return reply;
}

export function onCreateProgress(handler: (progress: CreateProgress) => void): Promise<UnlistenFn> {
  return listen<CreateProgress>(CREATE_PROGRESS_EVENT, (event) => handler(event.payload));
}

export function onInstanceExited(handler: (exited: InstanceExited) => void): Promise<UnlistenFn> {
  return listen<InstanceExited>(INSTANCE_EXITED_EVENT, (event) => handler(event.payload));
}
