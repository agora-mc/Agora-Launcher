/**
 * IPC wrappers for the INI and saves slice (the game's configuration files and save folder,
 * per instance, MASTER_SPEC §26.7).
 *
 * Every reply is checked against its shape before a caller sees it. A reply that is not the
 * documented shape rejects, so a caller never reads a missing answer as "no keys" or "no saves".
 */

import { invoke } from '@tauri-apps/api/core';
import { isGamesError } from '../shared/types';

/** Where the text of an INI file came from: the instance's copy, the game's own file, or nothing. */
export type IniSourceName = 'copy' | 'game_file' | 'nothing';

export interface IniFileRow {
  /** The file's name inside the instance, as the game names it (for example `Skyrim.ini`). */
  instancePath: string;
  /** Whether the instance has its own copy. A copy is made on the first save of a key. */
  copyExists: boolean;
  gameFile: string;
  gameFileExists: boolean;
}

export interface IniEntryRow {
  section: string;
  key: string;
  value: string;
}

export interface IniContents {
  instancePath: string;
  source: IniSourceName;
  entries: IniEntryRow[];
}

export interface IniView {
  files: IniFileRow[];
  /** Set when a file was named. Reading never creates the instance's copy. */
  contents: IniContents | null;
}

export interface IniChange {
  /** Whether the file actually changed. False when the key already had that value. */
  changed: boolean;
  view: IniView;
}

export type SaveChoice = 'own' | 'shared';

export interface SaveFolderRow {
  path: string;
  /** False when the folder has not been made yet. The game makes it. */
  exists: boolean;
  saves: number;
  newest: string | null;
}

export interface SavesView {
  instanceId: string;
  choice: SaveChoice;
  /** The folder the game reads and writes for this instance. */
  inUse: SaveFolderRow;
  /** The other folder. Its saves stay where they are when the choice changes. */
  other: SaveFolderRow;
  settingFile: string;
  /** `section/key` of the game setting that points at a folder. */
  setting: string;
  settingValue: string | null;
  settingSource: IniSourceName;
}

export interface SettingChangeRow {
  action: 'set' | 'unchanged' | 'restored' | 'removed' | 'left_alone' | 'nothing_recorded';
  value: string | null;
}

export interface SavesChange {
  previous: SaveChoice;
  setting: SettingChangeRow;
  saves: SavesView;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

function isIniSource(value: unknown): value is IniSourceName {
  return value === 'copy' || value === 'game_file' || value === 'nothing';
}

function isSaveChoice(value: unknown): value is SaveChoice {
  return value === 'own' || value === 'shared';
}

function isIniFileRow(value: unknown): value is IniFileRow {
  return isObject(value)
    && typeof value.instancePath === 'string'
    && typeof value.copyExists === 'boolean'
    && typeof value.gameFile === 'string'
    && typeof value.gameFileExists === 'boolean';
}

function isIniEntryRow(value: unknown): value is IniEntryRow {
  return isObject(value)
    && typeof value.section === 'string'
    && typeof value.key === 'string'
    && typeof value.value === 'string';
}

function isIniContents(value: unknown): value is IniContents {
  return isObject(value)
    && typeof value.instancePath === 'string'
    && isIniSource(value.source)
    && Array.isArray(value.entries)
    && value.entries.every(isIniEntryRow);
}

function isIniView(value: unknown): value is IniView {
  return isObject(value)
    && Array.isArray(value.files)
    && value.files.every(isIniFileRow)
    && (value.contents === null || isIniContents(value.contents));
}

function isIniChange(value: unknown): value is IniChange {
  return isObject(value) && typeof value.changed === 'boolean' && isIniView(value.view);
}

function isSaveFolderRow(value: unknown): value is SaveFolderRow {
  return isObject(value)
    && typeof value.path === 'string'
    && typeof value.exists === 'boolean'
    && typeof value.saves === 'number'
    && isNullableString(value.newest);
}

function isSavesView(value: unknown): value is SavesView {
  return isObject(value)
    && typeof value.instanceId === 'string'
    && isSaveChoice(value.choice)
    && isSaveFolderRow(value.inUse)
    && isSaveFolderRow(value.other)
    && typeof value.settingFile === 'string'
    && typeof value.setting === 'string'
    && isNullableString(value.settingValue)
    && isIniSource(value.settingSource);
}

function isSavesChange(value: unknown): value is SavesChange {
  return isObject(value)
    && isSaveChoice(value.previous)
    && isObject(value.setting)
    && typeof value.setting.action === 'string'
    && isNullableString(value.setting.value)
    && isSavesView(value.saves);
}

/** Returns the reply when it has the documented shape, and rejects by command name when it does not. */
function checkedReply<T>(command: string, reply: unknown, ok: (value: unknown) => value is T): T {
  if (!ok(reply)) {
    throw new Error(`${command} did not return the expected INI and saves data.`);
  }
  return reply;
}

/** The words to show for a failed command: the backend's own message when it sent one. */
export function describeFailure(error: unknown): string {
  if (isGamesError(error)) return error.message;
  return 'Agora could not finish that. Try again.';
}

/**
 * The instance's INI files. Pass `file` to read that file's keys as well. Reading never makes the
 * instance's copy.
 */
export async function getIni(instanceId: string, file: string | null = null): Promise<IniView> {
  const reply = await invoke<unknown>('games_ini_get', { instanceId, file });
  return checkedReply('games_ini_get', reply, isIniView);
}

/**
 * Set a key in one of the instance's INI files. A `null` value removes the key, so the game uses
 * its own default for it.
 */
export async function setIniValue(
  instanceId: string,
  file: string,
  section: string,
  key: string,
  value: string | null,
): Promise<IniChange> {
  const reply = await invoke<unknown>('games_ini_set', { instanceId, file, section, key, value });
  return checkedReply('games_ini_set', reply, isIniChange);
}

/** The instance's save choice, its two folders, and the game's setting that points at one. */
export async function getSaves(instanceId: string): Promise<SavesView> {
  const reply = await invoke<unknown>('games_saves_get', { instanceId });
  return checkedReply('games_saves_get', reply, isSavesView);
}

/** Make the instance's save choice `own` or `shared`. Never moves, copies or deletes a save. */
export async function setSaves(instanceId: string, choice: SaveChoice): Promise<SavesChange> {
  const reply = await invoke<unknown>('games_saves_set', { instanceId, choice });
  return checkedReply('games_saves_set', reply, isSavesChange);
}
