/**
 * IPC wrappers for the plugins slice (an instance's plugin list, load order, locks and checks).
 *
 * Every reply is checked against its shape before a caller sees it. A reply that is not the
 * documented shape rejects, so a caller never reads a missing answer as "no plugins" or "no
 * findings" (MASTER_SPEC §26.6).
 */

import { invoke } from '@tauri-apps/api/core';
import { isGamesError } from '../shared/types';

export interface PluginRow {
  /** 1-based, counting the always-loaded plugins. The number `move` takes. */
  position: number;
  name: string;
  /** Always loaded by the game. Not in the instance's list, so it cannot be moved or locked. */
  implicit: boolean;
  active: boolean;
  /** Whether the plugin's file is where the game looks for it. */
  present: boolean;
  /** Whether Agora added this line because a content layer deploys it. */
  managed: boolean;
  locked: boolean;
  headerRead: boolean;
  master: boolean;
  light: boolean;
  masters: string[];
  headerError: string | null;
}

export interface PluginFinding {
  message: string;
  /** Whether a launch refuses to start because of this finding. The others are warnings. */
  refusesLaunch: boolean;
  /** The plugins the finding is about. Empty for a limit on the whole list. */
  plugins: string[];
}

export interface PluginsView {
  /** False when the game keeps no plugin list. The tab says so instead of showing an empty list. */
  supported: boolean;
  /** Whether the game's load order rules (masters, light and full limits) apply. */
  rulesApply: boolean;
  /** False until the instance has a copy of the list, which is made at deploy time. */
  listExists: boolean;
  rows: PluginRow[];
  findings: PluginFinding[];
}

export interface PluginChange {
  /** Whether the line or the lock actually changed. */
  changed: boolean;
  plugins: PluginsView;
}

export interface MoveReport {
  plugin: string;
  from: number;
  to: number;
  written: boolean;
}

export interface MoveChange {
  report: MoveReport;
  plugins: PluginsView;
}

export interface SortMove {
  plugin: string;
  from: number;
  to: number;
}

export interface SortBlocked {
  plugin: string;
  master: string;
}

export interface SortReport {
  moves: SortMove[];
  /** Plugins that load above a master the sort may not move. The sort cannot fix those. */
  blocked: SortBlocked[];
  /** False for a dry run, which writes nothing. */
  written: boolean;
}

export interface SortChange {
  report: SortReport;
  plugins: PluginsView;
}

export interface CheckView {
  /** `findings` when a launch would refuse, `warnings` when there is something to read, `ok` otherwise. */
  status: 'ok' | 'warnings' | 'findings';
  findings: PluginFinding[];
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === 'string');
}

function isPluginRow(value: unknown): value is PluginRow {
  if (!isObject(value)) return false;
  return typeof value.position === 'number'
    && typeof value.name === 'string'
    && typeof value.implicit === 'boolean'
    && typeof value.active === 'boolean'
    && typeof value.present === 'boolean'
    && typeof value.managed === 'boolean'
    && typeof value.locked === 'boolean'
    && typeof value.headerRead === 'boolean'
    && typeof value.master === 'boolean'
    && typeof value.light === 'boolean'
    && isStringArray(value.masters)
    && (value.headerError === null || typeof value.headerError === 'string');
}

function isPluginFinding(value: unknown): value is PluginFinding {
  if (!isObject(value)) return false;
  return typeof value.message === 'string'
    && typeof value.refusesLaunch === 'boolean'
    && isStringArray(value.plugins);
}

function isPluginsView(value: unknown): value is PluginsView {
  if (!isObject(value)) return false;
  return typeof value.supported === 'boolean'
    && typeof value.rulesApply === 'boolean'
    && typeof value.listExists === 'boolean'
    && Array.isArray(value.rows)
    && value.rows.every(isPluginRow)
    && Array.isArray(value.findings)
    && value.findings.every(isPluginFinding);
}

function isPluginChange(value: unknown): value is PluginChange {
  return isObject(value) && typeof value.changed === 'boolean' && isPluginsView(value.plugins);
}

function isMoveChange(value: unknown): value is MoveChange {
  return isObject(value)
    && isObject(value.report)
    && typeof value.report.plugin === 'string'
    && typeof value.report.from === 'number'
    && typeof value.report.to === 'number'
    && typeof value.report.written === 'boolean'
    && isPluginsView(value.plugins);
}

function isSortChange(value: unknown): value is SortChange {
  return isObject(value)
    && isObject(value.report)
    && Array.isArray(value.report.moves)
    && Array.isArray(value.report.blocked)
    && typeof value.report.written === 'boolean'
    && isPluginsView(value.plugins);
}

function isCheckView(value: unknown): value is CheckView {
  return isObject(value)
    && (value.status === 'ok' || value.status === 'warnings' || value.status === 'findings')
    && Array.isArray(value.findings)
    && value.findings.every(isPluginFinding);
}

/** Returns the reply when it has the documented shape, and rejects by command name when it does not. */
function checkedReply<T>(command: string, reply: unknown, ok: (value: unknown) => value is T): T {
  if (!ok(reply)) {
    throw new Error(`${command} did not return the expected plugin data.`);
  }
  return reply;
}

/** An instance's plugins in load order, with what the game's rules say about them. */
export async function listGamePlugins(instanceId: string): Promise<PluginsView> {
  const reply = await invoke<unknown>('games_plugins_list', { instanceId });
  return checkedReply('games_plugins_list', reply, isPluginsView);
}

/** Turn a plugin on or off in the instance's list. Refused while the game is running. */
export async function setGamePluginActive(instanceId: string, plugin: string, active: boolean): Promise<PluginChange> {
  const reply = await invoke<unknown>('games_plugins_set_active', { instanceId, plugin, active });
  return checkedReply('games_plugins_set_active', reply, isPluginChange);
}

/** Lock or unlock a plugin's place, so sort and move leave it where it is. */
export async function setGamePluginLocked(instanceId: string, plugin: string, locked: boolean): Promise<PluginChange> {
  const reply = await invoke<unknown>('games_plugins_set_locked', { instanceId, plugin, locked });
  return checkedReply('games_plugins_set_locked', reply, isPluginChange);
}

/** Move a plugin to a 1-based position that counts the always-loaded plugins. */
export async function moveGamePlugin(instanceId: string, plugin: string, to: number): Promise<MoveChange> {
  const reply = await invoke<unknown>('games_plugins_move', { instanceId, plugin, to });
  return checkedReply('games_plugins_move', reply, isMoveChange);
}

/** Sort by the game's master rules. A dry run reports the moves and writes nothing. */
export async function sortGamePlugins(instanceId: string, dryRun: boolean): Promise<SortChange> {
  const reply = await invoke<unknown>('games_plugins_sort', { instanceId, dryRun });
  return checkedReply('games_plugins_sort', reply, isSortChange);
}

/** The load order's findings, read again. Read only. */
export async function checkGamePlugins(instanceId: string): Promise<CheckView> {
  const reply = await invoke<unknown>('games_plugins_check', { instanceId });
  return checkedReply('games_plugins_check', reply, isCheckView);
}

/** The words to show for a failed command: the backend's own message when it sent one. */
export function describeFailure(error: unknown): string {
  if (isGamesError(error)) return error.message;
  return 'Agora could not finish that. Try again.';
}
