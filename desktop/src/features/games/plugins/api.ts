/**
 * IPC wrappers for the plugins slice (an instance's plugin list, load order, locks and checks).
 * Every call is a stub until the slice lands.
 */

import { invoke } from '@tauri-apps/api/core';

export const listGamePlugins = (instanceId: string) => invoke<unknown>('games_plugins_list', { instanceId });
export const setGamePluginActive = (instanceId: string, plugin: string, active: boolean) =>
  invoke<unknown>('games_plugins_set_active', { instanceId, plugin, active });
export const setGamePluginLocked = (instanceId: string, plugin: string, locked: boolean) =>
  invoke<unknown>('games_plugins_set_locked', { instanceId, plugin, locked });
export const moveGamePlugin = (instanceId: string, plugin: string, toIndex: number) =>
  invoke<unknown>('games_plugins_move', { instanceId, plugin, toIndex });
export const sortGamePlugins = (instanceId: string) => invoke<unknown>('games_plugins_sort', { instanceId });
export const checkGamePlugins = (instanceId: string) => invoke<unknown>('games_plugins_check', { instanceId });
