/**
 * IPC wrappers for the tools slice (external programs that write into an instance, with their
 * output tracked). Every call is a stub until the slice lands.
 */

import { invoke } from '@tauri-apps/api/core';

export const listGameTools = (instanceId: string) => invoke<unknown>('games_tools_list', { instanceId });
export const runGameTool = (instanceId: string, toolId: string) =>
  invoke<unknown>('games_tools_run', { instanceId, toolId });
export const rollbackGameTool = (instanceId: string, toolId: string) =>
  invoke<unknown>('games_tools_rollback', { instanceId, toolId });
export const removeGameTool = (instanceId: string, toolId: string) =>
  invoke<unknown>('games_tools_remove', { instanceId, toolId });
export const diffGameTool = (instanceId: string, toolId: string) =>
  invoke<unknown>('games_tools_diff', { instanceId, toolId });
export const gameToolSwapCosts = (instanceId: string, toolId: string) =>
  invoke<unknown>('games_tools_swap_costs', { instanceId, toolId });
export const gameToolSwapStatus = (instanceId: string, toolId: string) =>
  invoke<unknown>('games_tools_swap_status', { instanceId, toolId });
export const restoreGameToolSwap = (instanceId: string, toolId: string) =>
  invoke<unknown>('games_tools_swap_restore', { instanceId, toolId });
