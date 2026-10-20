/**
 * IPC wrappers for the user-files slice (the game's own configs and saves that Agora backs up
 * and can restore). Every call is a stub until the slice lands.
 */

import { invoke } from '@tauri-apps/api/core';

export const userFilesStatus = (gameId: string) => invoke<unknown>('games_user_files_status', { gameId });
export const restoreUserFile = (gameId: string, backupId: string) =>
  invoke<unknown>('games_user_files_restore', { gameId, backupId });
