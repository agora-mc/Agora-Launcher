/**
 * IPC wrappers for the instances slice (the game instance list, an instance's page, its
 * checks and launch). Every call is a stub until the slice lands.
 */

import { invoke } from '@tauri-apps/api/core';

export const listGameInstances = () => invoke<unknown>('games_instance_list');
export const getGameInstance = (instanceId: string) => invoke<unknown>('games_instance_get', { instanceId });
export const createGameInstance = (gameId: string, baseId: string, name: string) =>
  invoke<unknown>('games_instance_create', { gameId, baseId, name });
export const deleteGameInstance = (instanceId: string) => invoke<unknown>('games_instance_delete', { instanceId });
export const checkGameInstance = (instanceId: string) => invoke<unknown>('games_instance_check', { instanceId });
export const setGameInstanceDeployment = (instanceId: string, mode: string) =>
  invoke<unknown>('games_instance_set_deployment', { instanceId, mode });
export const launchGameInstance = (instanceId: string) => invoke<unknown>('games_instance_launch', { instanceId });
