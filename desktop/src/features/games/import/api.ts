/**
 * IPC wrappers for the import slice (Mod Organizer 2 setups). Every call is a stub until the
 * slice lands.
 */

import { invoke } from '@tauri-apps/api/core';

export const scanMo2Setups = () => invoke<unknown>('games_import_mo2_scan');
export const importMo2Profile = (iniPath: string, profile: string) =>
  invoke<unknown>('games_import_mo2', { iniPath, profile });
