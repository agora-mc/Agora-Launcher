/**
 * IPC wrappers for the discovery slice (installs found on this machine, and the game folders the
 * user added by hand, MASTER_SPEC §26.3).
 *
 * Every call rejects on anything that is not the documented shape, so a caller never mistakes a
 * missing answer for "no games" or "no folders".
 */

import { invoke } from '@tauri-apps/api/core';
import type { GameListing, GamesList } from '../shared/types';

export type { GameListing, GamesList };

/** A folder the user added, and whether it is still on disk. */
export interface ListedGameFolder {
  path: string;
  present: boolean;
}

export interface ScanWarning {
  store: string;
  message: string;
}

/** What a fresh scan found besides the installs. */
export interface GameFolderScan {
  folders: ListedGameFolder[];
  warnings: ScanWarning[];
}

/** A folder core accepted: the game it holds, as Agora names it. */
export interface AddedGameFolder {
  path: string;
  gameId: string;
  gameName: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isGameListing(value: unknown): value is GameListing {
  if (!isRecord(value)) return false;
  return typeof value.gameId === 'string'
    && typeof value.name === 'string'
    && Array.isArray(value.installs);
}

function isScan(value: unknown): value is GameFolderScan {
  if (!isRecord(value) || !Array.isArray(value.folders) || !Array.isArray(value.warnings)) return false;
  const folders = value.folders.every((folder: unknown) =>
    isRecord(folder) && typeof folder.path === 'string' && typeof folder.present === 'boolean');
  const warnings = value.warnings.every((warning: unknown) =>
    isRecord(warning) && typeof warning.store === 'string' && typeof warning.message === 'string');
  return folders && warnings;
}

function isAddedFolder(value: unknown): value is AddedGameFolder {
  return isRecord(value)
    && typeof value.path === 'string'
    && typeof value.gameId === 'string'
    && typeof value.gameName === 'string';
}

/** Installs found on this machine, grouped by game. Rejects if the reply is not a list. */
export async function listInstalledGames(): Promise<GamesList> {
  const reply: unknown = await invoke<unknown>('games_list');
  if (
    !isRecord(reply)
    || !Array.isArray(reply.games)
    || !reply.games.every(isGameListing)
  ) {
    throw new Error('games_list did not return a list of games.');
  }
  return reply as unknown as GamesList;
}

/** Scan again: the game folders and the warnings. Rejects if the reply is not a scan. */
export async function discoverGames(): Promise<GameFolderScan> {
  const reply: unknown = await invoke<unknown>('games_discover');
  if (!isScan(reply)) {
    throw new Error('games_discover did not return the folders and warnings.');
  }
  return reply;
}

/** Add a folder the user picked. Rejects with the core's reason when the folder is refused. */
export async function addGameFolder(path: string): Promise<AddedGameFolder> {
  const reply: unknown = await invoke<unknown>('games_add_folder', { path });
  if (!isAddedFolder(reply)) {
    throw new Error('games_add_folder did not confirm the folder was added.');
  }
  return reply;
}

/** Stop searching a folder. Resolves with the stored path that was removed. */
export async function removeGameFolder(path: string): Promise<string> {
  const reply: unknown = await invoke<unknown>('games_remove_folder', { path });
  if (typeof reply !== 'string') {
    throw new Error('games_remove_folder did not confirm the folder was removed.');
  }
  return reply;
}
