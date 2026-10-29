/**
 * IPC wrappers for the user-files slice (the game's own configs and saves that Agora backs up
 * and can restore). Mirrors `desktop/src-tauri/src/games/user_files.rs`.
 */

import { invoke } from '@tauri-apps/api/core';

export interface SessionFile {
  realPath: string;
  /** The game's own file is currently replaced by Agora's copy. */
  swapped: boolean;
}

export interface UserFileSession {
  gameId: string;
  store: string;
  instanceId: string;
  /** The game is running on the swapped files, so a restore is refused. */
  running: boolean;
  files: SessionFile[];
}

export interface UserFilesStatus {
  sessions: UserFileSession[];
}

export interface RestoredFile {
  realPath: string;
  changed: boolean;
  originalSha256: string | null;
  finalSha256: string | null;
}

export interface RestoreResult {
  gameId: string;
  store: string;
  instanceId: string;
  files: RestoredFile[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** The file-swap sessions for a game. Rejects on a reply that is not a list of sessions. */
export async function userFilesStatus(gameId: string): Promise<UserFilesStatus> {
  const reply: unknown = await invoke<unknown>('games_user_files_status', { gameId });
  if (!isRecord(reply) || !Array.isArray(reply.sessions)) {
    throw new Error('games_user_files_status did not return a list of sessions.');
  }
  return reply as unknown as UserFilesStatus;
}

/** Put one session's files back from Agora's backup. */
export async function restoreUserFiles(gameId: string, store: string): Promise<RestoreResult> {
  const reply: unknown = await invoke<unknown>('games_user_files_restore', { gameId, store });
  if (!isRecord(reply) || !Array.isArray(reply.files)) {
    throw new Error('games_user_files_restore did not return a restore report.');
  }
  return reply as unknown as RestoreResult;
}
