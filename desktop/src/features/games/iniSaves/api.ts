/**
 * IPC wrappers for the INI and saves slice (the game's configuration files and save folder,
 * per instance). Every call is a stub until the slice lands.
 */

import { invoke } from '@tauri-apps/api/core';

export const getIniValue = (instanceId: string, file: string, section: string, key: string) =>
  invoke<unknown>('games_ini_get', { instanceId, file, section, key });
export const setIniValue = (instanceId: string, file: string, section: string, key: string, value: string) =>
  invoke<unknown>('games_ini_set', { instanceId, file, section, key, value });
export const getSaves = (instanceId: string) => invoke<unknown>('games_saves_get', { instanceId });
export const setSaves = (instanceId: string, folder: string) =>
  invoke<unknown>('games_saves_set', { instanceId, folder });
