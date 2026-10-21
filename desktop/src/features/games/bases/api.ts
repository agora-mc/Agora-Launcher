/**
 * IPC wrappers for the bases slice (pinned copies of a game that instances are built from).
 * Every call is a stub until the slice lands; each rejects with `not_implemented`.
 */

import { invoke } from '@tauri-apps/api/core';

export const listBases = (gameId: string) => invoke<unknown>('games_base_list', { gameId });
export const buildBase = (installId: string) => invoke<unknown>('games_base_build', { installId });
export const verifyBase = (baseId: string) => invoke<unknown>('games_base_verify', { baseId });
export const removeBase = (baseId: string) => invoke<unknown>('games_base_remove', { baseId });
