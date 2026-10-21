/**
 * IPC wrappers for the discovery slice (installs found on this machine, custom folders).
 *
 * `listInstalledGames` is the one real call. It rejects on anything that is not the
 * documented shape, so a caller never mistakes a missing answer for "no games".
 */

import { invoke } from '@tauri-apps/api/core';
import type { GameListing, GamesList } from '../shared/types';

export type { GameListing, GamesList };

function isGameListing(value: unknown): value is GameListing {
  if (typeof value !== 'object' || value === null) return false;
  const game = value as Partial<GameListing>;
  return typeof game.gameId === 'string'
    && typeof game.name === 'string'
    && Array.isArray(game.installs);
}

/** Installs found on this machine, grouped by game. Rejects if the reply is not a list. */
export async function listInstalledGames(): Promise<GamesList> {
  const reply: unknown = await invoke<unknown>('games_list');
  if (
    typeof reply !== 'object'
    || reply === null
    || !Array.isArray((reply as { games?: unknown }).games)
    || !(reply as GamesList).games.every(isGameListing)
  ) {
    throw new Error('games_list did not return a list of games.');
  }
  return reply as GamesList;
}

/** Stub until the discovery slice lands: scan again and return the warnings. */
export const discoverGames = () => invoke<unknown>('games_discover');

/** Stub until the discovery slice lands: search a folder for installs stores do not know. */
export const addGameFolder = (path: string) => invoke<unknown>('games_add_folder', { path });

/** Stub until the discovery slice lands: stop searching a folder. */
export const removeGameFolder = (path: string) => invoke<unknown>('games_remove_folder', { path });
