/**
 * IPC wrappers for the content slice (the content store, and what each instance has enabled,
 * in load order). Every call is a stub until the slice lands.
 */

import { invoke } from '@tauri-apps/api/core';

export const listContentStore = (gameId: string) => invoke<unknown>('games_content_list', { gameId });
export const addContentToStore = (gameId: string, archivePath: string) =>
  invoke<unknown>('games_content_add', { gameId, archivePath });
export const verifyStoredContent = (contentId: string) => invoke<unknown>('games_content_verify', { contentId });
export const removeStoredContent = (contentId: string) => invoke<unknown>('games_content_remove', { contentId });
export const listInstanceContent = (instanceId: string) =>
  invoke<unknown>('games_instance_content_list', { instanceId });
export const addInstanceContent = (instanceId: string, contentId: string) =>
  invoke<unknown>('games_instance_content_add', { instanceId, contentId });
export const removeInstanceContent = (instanceId: string, itemId: string) =>
  invoke<unknown>('games_instance_content_remove', { instanceId, itemId });
export const setInstanceContentEnabled = (instanceId: string, itemId: string, enabled: boolean) =>
  invoke<unknown>('games_instance_content_set_enabled', { instanceId, itemId, enabled });
export const moveInstanceContent = (instanceId: string, itemId: string, toIndex: number) =>
  invoke<unknown>('games_instance_content_move', { instanceId, itemId, toIndex });
