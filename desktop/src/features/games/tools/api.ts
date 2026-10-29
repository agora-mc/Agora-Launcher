/**
 * IPC wrappers for the tools slice (external programs such as Nemesis that write into an
 * instance, with their output tracked: MASTER_SPEC §26.9).
 *
 * The replies mirror `desktop/src-tauri/src/games/tools.rs`. Every read the screen acts on
 * (the list, a run's answer, the swap's costs and status) rejects a reply of any other shape, so
 * a missing answer is never shown as "no tools" or as a finished run.
 */

import { invoke } from '@tauri-apps/api/core';

/** Whether a tool's output is what the instance's inputs would build now. */
export type OutputStatus = 'current' | 'stale' | 'unknown' | 'conflict';

/** The capture a person picks on the screen. `swap` is never the default. */
export type CaptureChoice = 'auto' | 'vfs' | 'links' | 'swap';

/** What captured a run that ran. */
export type CaptureMethod = 'vfs' | 'links' | 'swap';

export interface ToolRow {
  toolId: string;
  name: string;
  /** Still declared by the game. False for output whose tool the game no longer declares. */
  declared: boolean;
  current: string | null;
  previous: string | null;
  /** `null` when the instance has no output for this tool yet. */
  status: OutputStatus | null;
  /** Another tool that conflicts with this one has enabled output as well. */
  conflicts: boolean;
}

export interface ToolsView {
  instanceId: string;
  tools: ToolRow[];
}

export interface SwapCosts {
  installDir: string;
  dataFiles: number;
  dataBytes: number;
  sameVolume: boolean;
  copyKind: 'kept' | 'per_run';
  copyFolder: string;
  keptCopyExists: boolean;
  copyFiles: number;
  copyBytes: number;
  /** `null` when the free space on the copy's volume cannot be read. */
  freeBytes: number | null;
  steamRunning: boolean;
}

export type ToolOffer = { offer: 'copySwap'; costs: SwapCosts } | { offer: 'switchToLinks' };

export interface InstallChanges {
  created: string[];
  changed: string[];
  deleted: string[];
  mayBeIncomplete: boolean;
  unavailable: string | null;
}

export interface SwapReport {
  copy: 'kept' | 'per_run';
  copiedInFiles: number;
  copiedInBytes: number;
  capturedFiles: number;
  deletedFiles: number;
  refreshedFiles: number;
  refreshedBytes: number;
}

export interface RunReport {
  toolId: string;
  name: string;
  run: string;
  capture: CaptureMethod;
  captureReason: string | null;
  promoted: boolean;
  current: string | null;
  previous: string | null;
  written: string[];
  deleted: string[];
  exitCode: number | null;
  cancelled: boolean;
  failedFolder: string | null;
  failure: string | null;
  copiedLayer: string | null;
  installChanges: InstallChanges;
  swap: SwapReport | null;
  offers: ToolOffer[];
  unchangedDropped: number;
}

/** What a run answered: a finished run, or a question that nothing ran for. */
export type RunResult =
  | { status: 'finished'; outcome: RunReport }
  | { status: 'vfsRefused'; reason: string; next: 'links' | 'swap' | null; offers: ToolOffer[] }
  | { status: 'needsSwap'; tool: string };

export interface InterruptedSwap {
  /** `null` when the journal cannot be read, so the tool is unknown. */
  tool: string | null;
  running: boolean;
  unreadable: string | null;
}

export interface SwapStatus {
  interrupted: InterruptedSwap | null;
}

export interface DiffReport {
  toolId: string;
  previous: string;
  current: string;
  added: string[];
  changed: string[];
  removed: string[];
}

export interface RemoveReply {
  removedGenerations: string[];
  view: ToolsView;
}

export interface RestoreReply {
  restoredTools: string[];
}

const isString = (value: unknown): value is string => typeof value === 'string';
const isNullableString = (value: unknown): value is string | null => value === null || typeof value === 'string';
const isStringArray = (value: unknown): value is string[] => Array.isArray(value) && value.every(isString);
const isNumber = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value);
const isNullableNumber = (value: unknown): value is number | null => value === null || isNumber(value);
const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const OUTPUT_STATUSES: readonly string[] = ['current', 'stale', 'unknown', 'conflict'];

function isToolRow(value: unknown): value is ToolRow {
  if (!isObject(value)) return false;
  return isString(value.toolId)
    && isString(value.name)
    && typeof value.declared === 'boolean'
    && isNullableString(value.current)
    && isNullableString(value.previous)
    && (value.status === null || (isString(value.status) && OUTPUT_STATUSES.includes(value.status)))
    && typeof value.conflicts === 'boolean';
}

export function isToolsView(value: unknown): value is ToolsView {
  return isObject(value)
    && isString(value.instanceId)
    && Array.isArray(value.tools)
    && value.tools.every(isToolRow);
}

export function isSwapCosts(value: unknown): value is SwapCosts {
  if (!isObject(value)) return false;
  return isString(value.installDir)
    && isNumber(value.dataFiles)
    && isNumber(value.dataBytes)
    && typeof value.sameVolume === 'boolean'
    && (value.copyKind === 'kept' || value.copyKind === 'per_run')
    && isString(value.copyFolder)
    && typeof value.keptCopyExists === 'boolean'
    && isNumber(value.copyFiles)
    && isNumber(value.copyBytes)
    && isNullableNumber(value.freeBytes)
    && typeof value.steamRunning === 'boolean';
}

function isOffer(value: unknown): value is ToolOffer {
  if (!isObject(value)) return false;
  if (value.offer === 'switchToLinks') return true;
  return value.offer === 'copySwap' && isSwapCosts(value.costs);
}

function isRunReport(value: unknown): value is RunReport {
  if (!isObject(value)) return false;
  const changes = value.installChanges;
  return isString(value.toolId)
    && isString(value.name)
    && isString(value.run)
    && (value.capture === 'vfs' || value.capture === 'links' || value.capture === 'swap')
    && isNullableString(value.captureReason)
    && typeof value.promoted === 'boolean'
    && isNullableString(value.current)
    && isNullableString(value.previous)
    && isStringArray(value.written)
    && isStringArray(value.deleted)
    && isNullableNumber(value.exitCode)
    && typeof value.cancelled === 'boolean'
    && isNullableString(value.failedFolder)
    && isNullableString(value.failure)
    && isNullableString(value.copiedLayer)
    && isObject(changes)
    && isStringArray(changes.created)
    && isStringArray(changes.changed)
    && isStringArray(changes.deleted)
    && typeof changes.mayBeIncomplete === 'boolean'
    && isNullableString(changes.unavailable)
    && (value.swap === null || isObject(value.swap))
    && Array.isArray(value.offers)
    && value.offers.every(isOffer)
    && isNumber(value.unchangedDropped);
}

export function isRunResult(value: unknown): value is RunResult {
  if (!isObject(value)) return false;
  if (value.status === 'finished') return isRunReport(value.outcome);
  if (value.status === 'vfsRefused') {
    return isString(value.reason)
      && (value.next === null || value.next === 'links' || value.next === 'swap')
      && Array.isArray(value.offers)
      && value.offers.every(isOffer);
  }
  if (value.status === 'needsSwap') return isString(value.tool);
  return false;
}

export function isSwapStatus(value: unknown): value is SwapStatus {
  if (!isObject(value)) return false;
  if (value.interrupted === null) return true;
  const swap = value.interrupted;
  return isObject(swap)
    && isNullableString(swap.tool)
    && typeof swap.running === 'boolean'
    && isNullableString(swap.unreadable);
}

function isDiffReport(value: unknown): value is DiffReport {
  return isObject(value)
    && isString(value.toolId)
    && isString(value.previous)
    && isString(value.current)
    && isStringArray(value.added)
    && isStringArray(value.changed)
    && isStringArray(value.removed);
}

function isRemoveReply(value: unknown): value is RemoveReply {
  return isObject(value) && isStringArray(value.removedGenerations) && isToolsView(value.view);
}

function isRestoreReply(value: unknown): value is RestoreReply {
  return isObject(value) && isStringArray(value.restoredTools);
}

function expectReply<T>(command: string, value: unknown, guard: (value: unknown) => value is T): T {
  if (!guard(value)) {
    throw new Error(`${command} did not return the reply the screen expects.`);
  }
  return value;
}

/** The tools the instance's game declares, with the output each has here. Rejects a malformed reply. */
export async function listGameTools(instanceId: string): Promise<ToolsView> {
  const reply = await invoke<unknown>('games_tools_list', { instanceId });
  return expectReply('games_tools_list', reply, isToolsView);
}

/**
 * Run a tool. `refusal` is the reason the virtual file system gave, when the person has chosen the
 * tier that runs instead. `confirmSwap` must be true for a swap, and only once its costs were shown.
 */
export async function runGameTool(
  instanceId: string,
  toolId: string,
  capture: CaptureChoice,
  options: { refusal?: string; confirmSwap?: boolean } = {},
): Promise<RunResult> {
  const reply = await invoke<unknown>('games_tools_run', {
    instanceId,
    toolId,
    capture,
    refusal: options.refusal ?? null,
    confirmSwap: options.confirmSwap ?? false,
  });
  return expectReply('games_tools_run', reply, isRunResult);
}

/** Roll the tool's output back to its previous generation. Returns the instance's tools as they now are. */
export async function rollbackGameTool(instanceId: string, toolId: string): Promise<ToolsView> {
  const reply = await invoke<unknown>('games_tools_rollback', { instanceId, toolId });
  return expectReply('games_tools_rollback', reply, isToolsView);
}

/** Remove the tool's output and its generations. Returns the instance's tools as they now are. */
export async function removeGameTool(instanceId: string, toolId: string): Promise<RemoveReply> {
  const reply = await invoke<unknown>('games_tools_remove', { instanceId, toolId });
  return expectReply('games_tools_remove', reply, isRemoveReply);
}

/** The files the tool's latest output changed, against the generation before it. */
export async function diffGameTool(instanceId: string, toolId: string): Promise<DiffReport> {
  const reply = await invoke<unknown>('games_tools_diff', { instanceId, toolId });
  return expectReply('games_tools_diff', reply, isDiffReport);
}

/** What a full-copy swap of this instance would cost. Changes nothing. */
export async function gameToolSwapCosts(instanceId: string): Promise<SwapCosts> {
  const reply = await invoke<unknown>('games_tools_swap_costs', { instanceId });
  return expectReply('games_tools_swap_costs', reply, isSwapCosts);
}

/** Whether a swap of this instance was interrupted and is waiting to be put back. */
export async function gameToolSwapStatus(instanceId: string): Promise<SwapStatus> {
  const reply = await invoke<unknown>('games_tools_swap_status', { instanceId });
  return expectReply('games_tools_swap_status', reply, isSwapStatus);
}

/** Put back the real Data folder of an interrupted swap. */
export async function restoreGameToolSwap(instanceId: string): Promise<RestoreReply> {
  const reply = await invoke<unknown>('games_tools_swap_restore', { instanceId });
  return expectReply('games_tools_swap_restore', reply, isRestoreReply);
}
