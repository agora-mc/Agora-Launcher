/**
 * IPC wrappers for the content slice: the content store (the Library's archives and folders) and
 * what each instance has, in load order. Replies are typed to match `games/content.rs`.
 *
 * The list calls reject on a reply that is not the documented shape, so a caller never reads a
 * missing answer as "nothing stored" (the same rule the discovery slice follows).
 */

import { invoke } from '@tauri-apps/api/core';

export type SourceKind = 'archive' | 'folder' | 'fomod' | 'thunderstore' | 'mo2' | 'catalog' | 'unknown';

export interface StoreItem {
  itemId: string;
  name: string;
  fileCount: number;
  totalSize: number;
  sourceKind: SourceKind;
  sourceLabel: string;
  addedAtUnixMs: number;
}

export interface StoreList {
  items: StoreItem[];
}

export interface StoreAdded {
  status: 'added' | 'existing';
  item: StoreItem;
  objectsNew: number;
  objectsRestored: number;
}

export type VerifyProblemKind = 'missing' | 'size_mismatch' | 'hash_mismatch' | 'unprotected' | 'corrupt_manifest';

export interface VerifyProblem {
  path: string;
  kind: VerifyProblemKind;
  message: string;
}

export interface VerifyReport {
  itemId: string;
  checked: number;
  hashed: number;
  problems: VerifyProblem[];
}

export interface InstanceContentRow {
  position: number;
  itemId: string;
  layerId: string;
  name: string;
  sourceKind: SourceKind;
  sourceLabel: string;
  enabled: boolean;
  mountPath: string;
  sourcePath: string;
}

export interface InstanceContentList {
  instanceId: string;
  items: InstanceContentRow[];
}

export interface ThunderstoreAdded {
  packageId: string;
  version: string;
  summary: string;
  missingDependencies: string[];
}

export interface AddedToInstance {
  itemId: string;
  layerId: string;
  mountPath: string;
  sourcePath: string;
  placementReason: string | null;
  thunderstore: ThunderstoreAdded | null;
}

export interface NeedsInstaller {
  itemId: string;
  reason: string;
  topLevel: string[];
}

/** `needs_installer` is not a failure: the archive's installer asks questions first. */
export type InstanceAddOutcome =
  | ({ status: 'added' } & AddedToInstance)
  | ({ status: 'needs_installer' } & NeedsInstaller);

function hasItemList(value: unknown): value is { items: unknown[] } {
  return typeof value === 'object' && value !== null && Array.isArray((value as { items?: unknown }).items);
}

function isStoreItem(value: unknown): value is StoreItem {
  if (typeof value !== 'object' || value === null) return false;
  const item = value as Partial<StoreItem>;
  return typeof item.itemId === 'string' && typeof item.name === 'string' && typeof item.sourceLabel === 'string';
}

/** The content store. Rejects if the reply is not a list of items. */
export async function listStore(): Promise<StoreList> {
  const reply: unknown = await invoke<unknown>('games_content_list');
  if (!hasItemList(reply) || !reply.items.every(isStoreItem)) {
    throw new Error('games_content_list did not return a list of items.');
  }
  return reply as StoreList;
}

/** Add an archive or a folder to the store. A blank name keeps the item's own. */
export const addToStore = (sourcePath: string, name: string | null) =>
  invoke<StoreAdded>('games_content_add', { sourcePath, name });

/** Check a stored item. `full` hashes every file. */
export const verifyStored = (itemId: string, full: boolean) =>
  invoke<VerifyReport>('games_content_verify', { itemId, full });

/** Remove an item from the store. Refused while an instance uses it. */
export const removeStored = (itemId: string) => invoke<void>('games_content_remove', { itemId });

/** The content layers an instance has, in load order. Rejects if the reply is not that list. */
export async function listInstanceContent(instanceId: string): Promise<InstanceContentList> {
  const reply: unknown = await invoke<unknown>('games_instance_content_list', { instanceId });
  if (!hasItemList(reply) || typeof (reply as { instanceId?: unknown }).instanceId !== 'string') {
    throw new Error('games_instance_content_list did not return an instance content list.');
  }
  return reply as InstanceContentList;
}

/** Add a stored item to an instance, placed by the game's rules. */
export const addInstanceContent = (instanceId: string, itemId: string) =>
  invoke<InstanceAddOutcome>('games_instance_content_add', { instanceId, itemId });

export const removeInstanceContent = (instanceId: string, itemId: string) =>
  invoke<void>('games_instance_content_remove', { instanceId, itemId });

export const setInstanceContentEnabled = (instanceId: string, itemId: string, enabled: boolean) =>
  invoke<void>('games_instance_content_set_enabled', { instanceId, itemId, enabled });

/** Move an item to a 1-based position (1 is the lowest priority). */
export const moveInstanceContent = (instanceId: string, itemId: string, position: number) =>
  invoke<void>('games_instance_content_move', { instanceId, itemId, position });

/** Ask the user for an archive (null if they cancel). Uses the app's existing picker command. */
export const pickArchive = () => invoke<string | null>('pick_open_file', {
  title: 'Choose a mod archive',
  extensions: ['zip', '7z', 'rar'],
});

/** Ask the user for a folder (null if they cancel). Uses the app's existing picker command. */
export const pickFolder = () => invoke<string | null>('pick_directory', { title: 'Choose a mod folder' });

/** A size as a person reads it. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return 'unknown size';
  if (bytes < 1024) return `${bytes} bytes`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unit]}`;
}
