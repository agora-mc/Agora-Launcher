export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
/**
 * The modloader an instance uses. `vanilla` is the sentinel for "none" — an
 * empty string also occurs on older instances, so test for both.
 */
export type Loader = 'vanilla' | 'fabric' | 'forge' | 'neoforge' | 'quilt';

/**
 * What a piece of installed content is. **Singular** — `mod`, not `mods`.
 * This is the vocabulary `content.list`'s optional filter takes, and the value
 * `ContentItem.contentType` carries back.
 */
export type ContentType = 'mod' | 'resourcepack' | 'shader' | 'datapack' | 'world';

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
  list(instanceId: string, contentType?: ContentType): Promise<ContentItem[]>;
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
 * `message` is for a human.
 */
export interface PluginCallError extends Error {
  code: PluginErrorCode | string;
  detail?: string;
}

/**
 * The codes an `agora` call can reject with.
 *
 * Typed as a union you can branch on, with `string` still permitted on
 * `PluginCallError.code` so a newer host adding a code does not break your
 * build. The ones an author actually handles are `CAPABILITY_DENIED` (you did
 * not ask for it in the manifest), `INVALID_ARGUMENTS` and `TIMEOUT`.
 */
export type PluginErrorCode =
  | 'INVALID_MANIFEST'
  | 'INVALID_PACKAGE'
  | 'INCOMPATIBLE_API'
  | 'UNRESOLVED_DEPENDENCY'
  | 'DEPENDENCY_CYCLE'
  | 'DUPLICATE_CONTRIBUTION'
  | 'CAPABILITY_DENIED'
  | 'UNKNOWN_METHOD'
  | 'INVALID_ARGUMENTS'
  | 'NOT_ACTIVATED'
  | 'SCRIPT_ERROR'
  | 'TIMEOUT'
  | 'CANCELLED'
  | 'RESOURCE_EXHAUSTED'
  | 'NETWORK_DENIED'
  | 'CONFLICT'
  | 'OPERATION_FAILED'
  | 'INTERNAL';
/**
 * The low-level escape hatch that isn't one: `call` runs the same capability
 * checks as the typed helpers above. A method you hold no capability for
 * rejects here exactly as it would there.
 */
export function call(method: string, args?: Json, options?: { deadlineMs?: number }): Promise<Json>;

// ---------------------------------------------------------------------------
// What your exports return
// ---------------------------------------------------------------------------
//
// The types above describe what you can *call*. These describe what a page,
// panel, replacement, diagnostic or launch check must *return* — which is the
// half that is easy to get wrong, because a misspelled field is ignored rather
// than rejected and the result is a view that renders with something missing.
//
// Type your exports with these and `tsc` will catch that for you.

/** How the host colours something. Defaults to `neutral` everywhere. */
export type Tone = 'neutral' | 'info' | 'success' | 'warning' | 'danger';

/** One table cell. Text, a badge, or a yes/no — never markup. */
export type Cell =
  | { type: 'text'; text: string }
  | { type: 'badge'; text: string; tone?: Tone }
  /** Rendered as a check or a dash, not the word "true". */
  | { type: 'flag'; value: boolean };

export interface Column {
  label: string;
  /** `end` right-aligns, for numbers. Defaults to `start`. */
  align?: 'start' | 'end';
}

export interface Stat {
  label: string;
  /** Already formatted: the host does not round or localise for you. */
  value: string;
  hint?: string;
  tone?: Tone;
}

export interface ListItem {
  title: string;
  detail?: string;
  tone?: Tone;
}

/** A button inside your view. Pressing it calls `export` on your plugin. */
export interface ActionButton {
  id: string;
  label: string;
  export: string;
  args?: Json;
  tone?: Tone;
  /**
   * Ask the user this question first. The host adds a confirmation to anything
   * it knows to be destructive whether or not you set this.
   */
  confirm?: string;
}

/** One renderable unit of a view. */
export type ViewBlock =
  | { type: 'heading'; text: string }
  | { type: 'text'; text: string; tone?: Tone }
  | { type: 'stats'; items: Stat[] }
  | {
      type: 'table';
      columns: Column[];
      /** One array of cells per row, in column order. */
      rows: Cell[][];
      emptyMessage?: string;
    }
  | { type: 'list'; items: ListItem[] }
  | { type: 'status'; tone: Tone; title: string; message?: string }
  | { type: 'actions'; items: ActionButton[] }
  | { type: 'divider' };

/**
 * What a page, instance panel or replacement export returns.
 *
 * At most 200 blocks, and at most 500 rows in a table. Exceeding either is an
 * error rather than a truncation, so a view that loops while building itself
 * says so instead of freezing the window.
 */
export interface ViewModel {
  title?: string;
  subtitle?: string;
  blocks: ViewBlock[];
}

/** Why the plugin believes a finding. Shown to the user as plain text. */
export interface Evidence {
  label: string;
  value: string;
}

/**
 * A fix a diagnostic offers. The host re-validates and applies it through the
 * same services the GUI uses; a plugin proposes, it does not perform.
 */
export interface RepairProposal {
  id: string;
  title: string;
  description?: string;
  action: RepairAction;
}

/**
 * The closed set of repairs the host knows how to carry out.
 *
 * Closed on purpose: a plugin proposes one of these and the host re-validates
 * and performs it through the same services the GUI uses. There is no
 * free-form action, which is the friction that should exist before a plugin
 * gains a new way to change someone's game.
 */
export type RepairAction =
  | { action: 'disableContent'; instanceId: string; key: string }
  | { action: 'enableContent'; instanceId: string; key: string }
  | { action: 'pinContentUpdate'; instanceId: string; key: string }
  | { action: 'unpinContentUpdate'; instanceId: string; key: string }
  | { action: 'setJvmMemory'; instanceId: string; memoryMb: number }
  | { action: 'resetJvmArgs'; instanceId: string }
  | { action: 'createSnapshot'; instanceId: string; label?: string };

export type Severity = 'info' | 'warning' | 'error';

export interface Finding {
  /** Stable across runs, so the host can tell "still broken" from "broken again". */
  id: string;
  title: string;
  severity?: Severity;
  summary?: string;
  evidence?: Evidence[];
  repairs?: RepairProposal[];
}

/**
 * What a diagnostic **and a launch check** return. They share this shape.
 *
 * `incompleteReason` is for when you could not finish: a partial report with
 * an honest note beats a clean report that silently checked nothing.
 */
export interface DiagnosticReport {
  findings?: Finding[];
  incompleteReason?: string;
}

/**
 * The argument every instance-scoped export receives: instance panels,
 * instance-context commands, diagnostics and launch checks.
 */
export interface InstanceScope {
  instanceId: string;
}
