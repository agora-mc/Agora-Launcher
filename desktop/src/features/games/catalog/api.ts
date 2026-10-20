/**
 * IPC wrappers for the catalog slice (the curated catalog for games other than Minecraft).
 * Every call is a stub until the slice lands.
 */

import { invoke } from '@tauri-apps/api/core';

export const listCatalogEntries = (gameId: string) => invoke<unknown>('games_catalog_list', { gameId });
export const installCatalogEntry = (gameId: string, entryId: string) =>
  invoke<unknown>('games_catalog_install', { gameId, entryId });
