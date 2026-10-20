/**
 * IPC wrappers for the FOMOD slice (an archive's installer steps and its install).
 * Every call is a stub until the slice lands.
 */

import { invoke } from '@tauri-apps/api/core';

export const showFomod = (contentId: string) => invoke<unknown>('games_fomod_show', { contentId });
export const installFomod = (contentId: string, choices: unknown) =>
  invoke<unknown>('games_fomod_install', { contentId, choices });
