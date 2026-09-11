export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export interface Instance {
  id: string; name: string; minecraftVersion: string; loader: string; loaderVersion: string;
  isModpack: boolean; isLocked: boolean; lastLaunchedAt?: string; createdAt: string;
}
export interface InstanceDetail extends Instance {
  jvm: { memoryMb: number; memoryMode: string; gc: string; customArgs: string; alwaysPreTouch: boolean; hasJavaOverride: boolean };
  launchMode: string;
  packOrigin?: string;
  contentCounts: { mods: number; resourcepacks: number; shaders: number; datapacks: number; worlds: number; disabled: number };
}
export interface ContentItem {
  key: string; filename: string; displayName: string; enabled: boolean; contentType: string;
  version?: string; installedAt: string; sourceLabel: string; packManaged: boolean; installedAsDependency: boolean;
  updatePinned: boolean; filePresent: boolean; sizeBytes?: number; author?: string; categories?: string[];
  sourceUrl?: string; registryId?: string; modrinthId?: string;
}
export interface PluginEvent {
  sequence: number; event: string; instanceId?: string; operationId?: string; depth: number;
  origin: { kind: 'user' | 'system' } | { kind: 'plugin'; pluginId: string };
}
export interface Store {
  get(key: string): Promise<Json>;
  set(key: string, value: Json): Promise<{ stored: boolean }>;
  all(): Promise<Record<string, Json>>;
  remove(key: string): Promise<{ removed: boolean }>;
}
export const instances: {
  list(): Promise<Instance[]>;
  get(instanceId: string): Promise<InstanceDetail | null>;
  rename(instanceId: string, name: string): Promise<{ renamed: boolean }>;
  setMemory(instanceId: string, memoryMb: number): Promise<{ memoryMb: number }>;
};
export const content: {
  list(instanceId: string, contentType?: string): Promise<ContentItem[]>;
  enable(instanceId: string, key: string): Promise<{ key: string; enabled: boolean }>;
  disable(instanceId: string, key: string): Promise<{ key: string; enabled: boolean }>;
  setUpdatePinned(instanceId: string, key: string, pinned: boolean): Promise<{ key: string; pinned: boolean; changed: boolean }>;
};
export const storage: Store & { forInstance(instanceId: string): Store };
export interface LaunchState {
  instanceId: string;
  /** `idle`, `preparing`, `running`, `exited`, or `crashed`. */
  status: string;
  startedAt?: string;
  /** Present only for a direct launch that is actually running. */
  pid?: number;
}
export interface LaunchHistoryEntry {
  instanceId: string; startedAt: string; prepMs?: number; durationMs?: number;
  outcome?: string; enabledModCount: number; minecraftVersion: string; loader: string;
  peakMemoryMb?: number;
}
export const launch: {
  state(instanceId: string): Promise<LaunchState>;
  /** `limit` is clamped by the host to 1..200 regardless of what you pass. */
  history(instanceId: string, limit?: number): Promise<LaunchHistoryEntry[]>;
};
export const settings: Readonly<Record<string, Json>>;
export const apiVersion: string;
export const pluginId: string;
export type Tone = 'neutral' | 'info' | 'success' | 'warning' | 'danger';
export const ui: {
  refresh(viewId?: string): Promise<{ requested: boolean }>;
  /** Messages are capped at 200 characters and are attributed to your plugin. */
  notify(tone: Tone, message: string): Promise<{ shown: boolean }>;
};
export const net: {
  /**
   * GET JSON from a host you declared in `network.hosts`. Requires the `network`
   * capability, the user's network opt-in, and Lockdown Mode being off. Rejects
   * with `NETWORK_DENIED` otherwise. Responses above 2 MiB are refused.
   */
  fetchJson(url: string, options?: Record<string, Json>): Promise<{ status: number; body: Json }>;
};
export const log: { debug(...values: unknown[]): void; info(...values: unknown[]): void; warn(...values: unknown[]): void; error(...values: unknown[]): void };
export function on(eventName: string, handler: (event: PluginEvent) => void | Promise<void>): () => void;
export function off(eventName: string, handler: (event: PluginEvent) => void | Promise<void>): void;
/**
 * An error thrown by any `agora` call. `code` is the stable part to branch on;
 * `message` is for a human. See `docs/plugins/README.md` for the code list.
 */
export interface PluginCallError extends Error {
  code: string;
  detail?: string;
}
/**
 * The low-level escape hatch that isn't one: `call` runs the same capability
 * checks as the typed helpers above. A method you hold no capability for
 * rejects here exactly as it would there.
 */
export function call(method: string, args?: Json, options?: { deadlineMs?: number }): Promise<Json>;
