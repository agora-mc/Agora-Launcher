import { invoke } from '@tauri-apps/api/core';

/**
 * Format any thrown error (including Tauri's serialized LauncherError shape)
 * into a readable string for UI display.
 *
 * Tauri invokes reject with error values that depend on the Rust enum's serde
 * serialization:
 *   - Unit variants like `HashMismatch` come across as the string `"HashMismatch"`.
 *   - Struct variants like `Generic { code, message }` come across as
 *     `{ Generic: { code: "...", message: "..." } }` (serde's default
 *     externally-tagged representation).
 *
 * Plain JS `Error` objects also flow through here (`e.message` works).
 *
 * Using this helper instead of `String(e)` avoids the dreaded `[object Object]`.
 */
export function formatError(e: unknown): string {
  if (e == null) return 'Unknown error';
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  if (typeof e === 'object') {
    const obj = e as Record<string, unknown>;
    // New structured error envelope: { code, message, details, suggested_action }
    if (typeof obj.code === 'string' && typeof obj.message === 'string') {
      return obj.message;
    }
    // Tauri serialized struct variant: { VariantName: { code, message } }
    for (const key of Object.keys(obj)) {
      const inner = obj[key];
      if (inner && typeof inner === 'object') {
        const innerObj = inner as Record<string, unknown>;
        if (typeof innerObj.message === 'string') return innerObj.message;
        if (typeof innerObj.code === 'string') return innerObj.code;
      }
      if (typeof inner === 'string') return inner;
    }
    // Direct shape: { message: "..." } or { code: "..." }
    if (typeof obj.message === 'string') return obj.message;
    if (typeof obj.code === 'string') return obj.code;
    try {
      return JSON.stringify(e);
    } catch {
      return '[object]';
    }
  }
  return String(e);
}

// ---------------------------------------------------------------------------
// Structured error parsing — recoverable profile issues (Phase 5/Stage 5)
// ---------------------------------------------------------------------------

/**
 * The kind of profile issue encountered during direct-launch adoption.
 * Mirrors Rust `ProfileIssueKind` serde serialization (externally-tagged
 * string values: "MissingProfile", "UnsupportedProfileMetadata", "CorruptProfile").
 */
export type ProfileIssueKind =
  | 'MissingProfile'
  | 'UnsupportedProfileMetadata'
  | 'CorruptProfile';

/**
 * Structured recoverable issue extracted from a LauncherError details envelope.
 * Mirrors Rust `ProfileIssue` with the same field names (camelCase in JS).
 */
export interface RecoverableProfileIssue {
  kind: ProfileIssueKind;
  profile_path: string | null;
  reasons: string[];
}

/**
 * Available user actions for a recoverable profile issue.
 * Derived from Rust's SUGGEST_* constants in installed_profile.rs.
 *
 * Extended for Stage 4 Java recovery with download_runtime, choose_java,
 * open_privacy actions.
 */
export type LauncherAction =
  | 'reinstall_loader'
  | 'use_delegated_launch'
  | 'dismiss'
  | 'download_runtime'
  | 'choose_java'
  | 'cancel'
  | 'open_privacy';

/**
 * Structured recoverable Java issue extracted from a LauncherError details
 * envelope for ERR_JAVA_RUNTIME_MISSING or ERR_JAVA_RUNTIME_CATALOG_MISSING.
 */
export interface RecoverableJavaIssue {
  major: number;
  component?: string;
  os?: string;
  arch?: string;
}

/**
 * Parsed structured launcher error envelope.
 *
 * All errors (even non-recoverable ones) produce a ParsedLauncherError by
 * extracting code + message. Only profile-related errors populate
 * `recoverableIssue` and `availableActions`. Java runtime errors populate
 * `recoverableJavaIssue`.
 */
export interface ParsedLauncherError {
  code: string;
  message: string;
  recoverableIssue: RecoverableProfileIssue | null;
  recoverableJavaIssue: RecoverableJavaIssue | null;
  availableActions: LauncherAction[];
}

/**
 * Parse any thrown error into a structured ParsedLauncherError.
 *
 * Unlike `formatError` (which returns a plain string), this inspects the
 * Tauri-serialized error envelope without string-matching error text.
 *
 * The Rust backend serializes LauncherError as a map:
 *   { code: string, message: string, details: {...} | null, suggested_action: string | null }
 *
 * For profile issues (ProfileMissing, ProfileUnsupportedMetadata, ProfileCorrupt),
 * details contains:
 *   { recoverable_issue: { kind, profile_path, reasons }, suggested_actions: string[] }
 */
export function parseLauncherError(e: unknown): ParsedLauncherError {
  // Default fallback — no structured info.
  const fallback = (message: string): ParsedLauncherError => ({
    code: 'ERR_UNKNOWN',
    message,
    recoverableIssue: null,
    recoverableJavaIssue: null,
    availableActions: [],
  });

  if (e == null) return fallback('Unknown error');
  if (typeof e === 'string') return fallback(e);
  if (e instanceof Error) return fallback(e.message);

  if (typeof e === 'object') {
    const obj = e as Record<string, unknown>;

    // Check for the standard LauncherError envelope: { code, message, details }
    const code = typeof obj.code === 'string' ? obj.code : null;
    const message = typeof obj.message === 'string' ? obj.message : null;

    // If we have a code and message, try to extract recoverable details.
    if (code && message) {
      const details = obj.details;
      let recoverableIssue: RecoverableProfileIssue | null = null;
      let recoverableJavaIssue: RecoverableJavaIssue | null = null;
      let availableActions: LauncherAction[] = [];

      if (details && typeof details === 'object') {
        const det = details as Record<string, unknown>;
        const rawIssue = det.recoverable_issue;
        const rawActions = det.suggested_actions;
        const rawMajor = det.major;
        const rawComponent = det.component;
        const rawOs = det.os;
        const rawArch = det.arch;

        // Parse recoverable profile issue
        if (rawIssue && typeof rawIssue === 'object') {
          const ri = rawIssue as Record<string, unknown>;
          const kind = ri.kind;
          if (
            typeof kind === 'string' &&
            (kind === 'MissingProfile' || kind === 'UnsupportedProfileMetadata' || kind === 'CorruptProfile')
          ) {
            recoverableIssue = {
              kind: kind as ProfileIssueKind,
              profile_path: typeof ri.profile_path === 'string' ? ri.profile_path : null,
              reasons: Array.isArray(ri.reasons)
                ? ri.reasons.filter((r): r is string => typeof r === 'string')
                : [],
            };

            if (Array.isArray(rawActions)) {
              availableActions = rawActions.filter(
                (a): a is LauncherAction =>
                  a === 'reinstall_loader' || a === 'use_delegated_launch' || a === 'dismiss',
              );
            }
          }
        }

        // Parse recoverable Java issue (ERR_JAVA_RUNTIME_MISSING / ERR_JAVA_RUNTIME_CATALOG_MISSING)
        if (code === 'ERR_JAVA_RUNTIME_MISSING' && typeof rawMajor === 'number') {
          recoverableJavaIssue = {
            major: rawMajor,
            component: typeof rawComponent === 'string' ? rawComponent : undefined,
          };

          if (Array.isArray(rawActions)) {
            availableActions = rawActions.filter(
              (a): a is LauncherAction =>
                a === 'download_runtime' || a === 'choose_java' || a === 'cancel' || a === 'open_privacy',
            );
            // Default actions when backend doesn't provide them
            if (availableActions.length === 0) {
              availableActions = ['download_runtime', 'choose_java', 'cancel'];
            }
          }
        }

        if (code === 'ERR_JAVA_RUNTIME_CATALOG_MISSING' && typeof rawMajor === 'number') {
          recoverableJavaIssue = {
            major: rawMajor,
            os: typeof rawOs === 'string' ? rawOs : undefined,
            arch: typeof rawArch === 'string' ? rawArch : undefined,
          };

          if (Array.isArray(rawActions)) {
            availableActions = rawActions.filter(
              (a): a is LauncherAction =>
                a === 'choose_java' || a === 'cancel',
            );
            if (availableActions.length === 0) {
              availableActions = ['choose_java', 'cancel'];
            }
          }
        }

        // Parse ERR_JAVA_RUNTIME_DOWNLOAD_DISABLED
        if (code === 'ERR_JAVA_RUNTIME_DOWNLOAD_DISABLED' && typeof rawMajor === 'number') {
          recoverableJavaIssue = {
            major: rawMajor,
            component: typeof rawComponent === 'string' ? rawComponent : undefined,
          };
          if (Array.isArray(rawActions)) {
            availableActions = rawActions.filter(
              (a): a is LauncherAction =>
                a === 'choose_java' || a === 'open_privacy' || a === 'cancel',
            );
            if (availableActions.length === 0) {
              availableActions = ['choose_java', 'open_privacy', 'cancel'];
            }
          }
        }

        // Parse ERR_JAVA_RUNTIME_CANCELLED
        if (code === 'ERR_JAVA_RUNTIME_CANCELLED' && typeof rawMajor === 'number') {
          recoverableJavaIssue = {
            major: rawMajor,
            component: typeof rawComponent === 'string' ? rawComponent : undefined,
          };
          availableActions = ['cancel'];
        }

        // Also handle ERR_JAVA_RUNTIME_MISSING when the backend uses Generic envelope
        // (network-disabled path that preserves major via format string)
        if (!recoverableJavaIssue && rawActions === undefined) {
          if (code === 'ERR_JAVA_RUNTIME_MISSING' && typeof rawMajor === 'number') {
            // Raw generic error still carries major from the details
            recoverableJavaIssue = {
              major: rawMajor,
            };
            availableActions = ['choose_java', 'cancel'];
            // add open_privacy only when message mentions privacy
            if (message && message.toLowerCase().includes('privacy')) {
              availableActions.push('open_privacy');
            }
          }
        }
      }

      return { code, message, recoverableIssue, recoverableJavaIssue, availableActions };
    }

    // Try Tauri externally-tagged enum variant: { VariantName: { code, message } }
    for (const key of Object.keys(obj)) {
      const inner = obj[key];
      if (inner && typeof inner === 'object') {
        const innerObj = inner as Record<string, unknown>;
        if (typeof innerObj.code === 'string' && typeof innerObj.message === 'string') {
          return {
            code: innerObj.code as string,
            message: innerObj.message as string,
            recoverableIssue: null,
            recoverableJavaIssue: null,
            availableActions: [],
          };
        }
        if (typeof innerObj.message === 'string') {
          return fallback(innerObj.message as string);
        }
      }
      if (typeof inner === 'string') return fallback(inner);
    }

    // Bare { message } or { code }
    if (typeof obj.message === 'string') return fallback(obj.message);
    if (typeof obj.code === 'string') return fallback(obj.code);

    try {
      return fallback(JSON.stringify(e));
    } catch {
      return fallback('[object]');
    }
  }

  return fallback(String(e));
}

/** Check whether a thrown error is an expired-GitHub-session error. */
export function isAuthExpired(e: unknown): boolean {
  if (e == null || typeof e !== 'object') return false;
  const obj = e as Record<string, unknown>;
  if (obj.code === 'ERR_AUTH_EXPIRED') return true;
  // Tauri serialized struct variant: { AuthExpired: { code: "ERR_AUTH_EXPIRED", ... } }
  for (const key of Object.keys(obj)) {
    const inner = obj[key];
    if (inner && typeof inner === 'object') {
      const innerObj = inner as Record<string, unknown>;
      if (innerObj.code === 'ERR_AUTH_EXPIRED') return true;
    }
  }
  return false;
}

export interface InstanceRow {
  instance_id: string;
  name: string;
  minecraft_version: string;
  loader: string;
  loader_version: string;
  is_modpack: boolean;
  is_locked: boolean;
  last_launched_at: string | null;
  jvm_memory_mb: number;
  jvm_memory_mode: 'auto' | 'manual';
  jvm_gc: string;
  jvm_custom_args: string;
  jvm_always_pre_touch: boolean;
  created_at: string;
  java_path: string | null;
  java_incompatible_override: boolean;
  icon_path: string | null;
  launch_mode_override: 'auto' | 'direct' | 'delegated';
  import_source: string | null;
}

export interface InstalledMod {
  filename: string;
  registry_id: string | null;
  modrinth_id: string | null;
  source: string;
  source_url?: string | null;
  version: string | null;
  sha256: string;
  installed_at: string;
  mod_jar_id?: string | null;
  enabled: boolean;
  content_type: string;
  /** True when the modpack contributed this entry rather than the user. */
  pack_managed?: boolean;
  update_pinned?: boolean;
  /** True when Agora installed this only to satisfy another mod's dependency. */
  installed_as_dependency?: boolean;
}

export interface InstanceManifest {
  instance_id: string;
  name: string;
  created_from_pack: string | null;
  minecraft_version: string;
  loader: string;
  loader_version: string;
  is_locked: boolean;
  mods: InstalledMod[];
  resourcepacks: InstalledMod[];
  shaders: InstalledMod[];
  datapacks: InstalledMod[];
  worlds: InstalledMod[];
  user_preferences: Record<string, unknown>;
}

export interface InstanceDetail {
  row: InstanceRow;
  manifest: InstanceManifest | null;
  snapshot_readiness: 'ready' | 'pending' | 'failed';
  snapshot_error: string | null;
}

export type CurationStatus = 'curated' | 'under_review' | 'uncurated' | 'archived' | 'unknown';
export type MetadataStatus = 'complete' | 'partial' | 'unavailable';

export interface InstalledContentRow {
  key: string;
  filename: string;
  display_name: string;
  version: string | null;
  content_type: string;
  enabled: boolean;
  installed_at: string;
  source: string;
  source_label: string;
  /** True when the modpack contributed this entry rather than the user. */
  pack_managed: boolean;
  /** True when Agora installed this only to satisfy another mod's dependency. */
  installed_as_dependency: boolean;
  /** True when the user pinned this entry against updates. */
  update_pinned: boolean;
  source_url: string | null;
  registry_id: string | null;
  modrinth_id: string | null;
  mod_jar_id: string | null;
  loader_mod_id: string | null;
  size_bytes: number | null;
  file_present: boolean;
  resolved_path: string | null;
  author: string | null;
  categories: string[];
  icon_url: string | null;
  curation_status: CurationStatus;
  agora_score: number | null;
  modrinth_downloads: number | null;
  metadata_status: MetadataStatus;
}

export interface InstalledContentMetadata {
  key: string;
  display_name: string | null;
  icon_url: string | null;
  author: string | null;
}

export interface LoaderVersionSummary {
  loader: string;
  mc_version: string;
  loader_version: string;
  file_type: string;
}

/// One place a registry item's file can be fetched from.
export interface DownloadSource {
  strategy: string;
  identifier: string;
}

export interface RegistryItem {
  id: string;
  name: string;
  content_type: string;
  /// Preferred source's strategy. Mirrors `download_sources[0].strategy`;
  /// prefer `downloadSourcesOf(item)` when the fallbacks matter.
  download_strategy: string;
  source_identifier: string;
  sha256: string;
  upvotes: number;
  downvotes: number;
  net_score: number;
  velocity: number;
  status: string;
  is_immune: boolean;
  immunity_reason: string | null;
  allow_comments: boolean;
  icon_url: string | null;
  gallery_urls_json: string | null;
  date_added: string | null;
  compatible_versions_json: string | null;
  description: string | null;
  body_markdown: string | null;
  page_url: string | null;
  license_id: string | null;
  source_updated_at: string | null;
  modrinth_id: string | null;
  /// Ordered download sources, best first, as stored in the signed registry.
  /// Absent on registries compiled before the multi-source schema.
  download_sources_json?: string | null;
  recommendation_reason?: string | null;
  recommendation_overlap?: number | null;
}

/// Ordered download sources for an item, best first.
///
/// Mirrors the backend's fallback reconstruction so the UI shows the same list
/// the resolver will actually walk: a registry row without an explicit source
/// list still has its preferred source, plus the implicit Modrinth fallback a
/// `modrinth_id` gives it.
export function downloadSourcesOf(item: Pick<RegistryItem,
  'download_strategy' | 'source_identifier' | 'modrinth_id' | 'download_sources_json'>): DownloadSource[] {
  const raw = item.download_sources_json?.trim();
  if (raw) {
    try {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        const sources = parsed.filter(
          (entry): entry is DownloadSource =>
            !!entry && typeof entry.strategy === 'string' && entry.strategy.trim() !== '',
        );
        if (sources.length > 0) return sources;
      }
    } catch {
      // Malformed JSON falls through to the legacy reconstruction below.
    }
  }
  const sources: DownloadSource[] = [
    { strategy: item.download_strategy, identifier: item.source_identifier },
  ];
  const modrinthId = item.modrinth_id?.trim();
  if (modrinthId && item.download_strategy !== 'modrinth_id') {
    sources.push({ strategy: 'modrinth_id', identifier: modrinthId });
  }
  return sources;
}

/// Product names the generic word-capitalizer would get wrong.
const DOWNLOAD_SOURCE_LABELS: Record<string, string> = {
  github_release: 'GitHub Release',
  modrinth_id: 'Modrinth',
  direct_hash: 'Direct Download',
  technic_pack: 'Technic',
  curated_pack: 'Curated Pack',
};

/// Human-readable label for a download strategy (`github_release` → `GitHub Release`).
export function downloadSourceLabel(strategy: string): string {
  const known = DOWNLOAD_SOURCE_LABELS[strategy];
  if (known) return known;
  return strategy.replace(/_/g, ' ').replace(/\b\w/g, (letter) => letter.toUpperCase());
}

export interface CategoryInfo {
  id: string;
  display_name: string;
  is_community: boolean;
  content_types: string[];
}

export type SortOption = 'for_you' | 'net_score' | 'velocity' | 'most_downvoted' | 'newest' | 'most_upvoted';

export interface RegistryStatus {
  has_cached_db: boolean;
  cached_tag: string | null;
  cached_schema_version: number | null;
  latest_tag: string | null;
  update_available: boolean;
  checked: boolean;
  message: string;
}

export interface ExtractionResult {
  extracted: string[];
  skipped: string[];
  total_bytes_written: number;
}

export interface CreateInstanceRequest {
  name: string;
  instance_id: string;
  minecraft_version: string;
  loader: string;
  loader_version: string;
  jvm_memory_mb?: number;
  jvm_memory_mode?: 'auto' | 'manual';
  jvm_gc?: string;
  jvm_custom_args?: string;
  is_modpack?: boolean;
  pack_icon_url?: string | null;
  /** Instance template to seed configs and JVM settings from. Omit to use the
   *  stored default template (if any); explicit request fields always win. */
  template_id?: string | null;
}

export interface PackModRow {
  pack_id: string;
  mod_id: string;
  source: string;
  version: string | null;
  status: string;
  description: string | null;
}

export const listPackMods = (packId: string) =>
  invoke<PackModRow[]>('list_pack_mods', { packId });

export const listInstances = () => invoke<InstanceRow[]>('list_instances');
export const getInstanceDetail = (instanceId: string) =>
  invoke<InstanceDetail | null>('get_instance_detail', { instanceId });
export const listInstanceContent = (instanceId: string, contentType?: string) =>
  invoke<InstalledContentRow[]>('list_instance_content', {
    instanceId,
    contentType: contentType ?? null,
  });
export const enrichInstanceContent = (instanceId: string) =>
  invoke<InstalledContentMetadata[]>('enrich_instance_content', { instanceId });
export const createInstance = (request: CreateInstanceRequest) =>
  invoke<InstanceRow>('create_instance', { request });
export const deleteInstance = (instanceId: string) =>
  invoke<void>('delete_instance', { instanceId });
export const unlockInstance = (instanceId: string) =>
  invoke<void>('unlock_instance', { instanceId });
export const lockInstance = (instanceId: string) =>
  invoke<void>('lock_instance', { instanceId });
export const renameInstance = (instanceId: string, newName: string) =>
  invoke<void>('rename_instance', { instanceId, newName });
export const revertInstance = (instanceId: string) =>
  invoke<void>('revert_instance', { instanceId });
export const openInstanceFolder = (instanceId: string) =>
  invoke<void>('open_instance_folder', { instanceId });
export const openDataFolder = () => invoke<void>('open_data_folder');
/** Relaunch the app. Used after the updater stages a new version. Never resolves. */
export const restartApp = () => invoke<void>('restart_app');
export const revealPath = (path: string) =>
  invoke<void>('reveal_path', { path });

/** Open an external https link in the user's real browser. */
export const openExternalUrl = (url: string) =>
  invoke<void>('open_external_url', { url });
export const launchInstance = (instanceId: string, allowHealthBlockers = false, healthScanToken?: string) =>
  invoke<void>('launch_instance', { instanceId, allowHealthBlockers, healthScanToken: healthScanToken ?? null });

export const launchInstanceDirect = (instanceId: string, allowHealthBlockers = false, healthScanToken?: string) =>
  invoke<number>('launch_instance_direct', { instanceId, allowHealthBlockers, healthScanToken: healthScanToken ?? null });

/**
 * Coarse recovery action for launch_instance_with_recovery.
 * Mirrors Rust `LaunchRecoveryAction` serde (internally-tagged enum).
 */
export type LaunchRecoveryAction =
  | { type: 'None' }
  | { type: 'ProvisionJava'; major: number }
  | { type: 'RepairLoader' }
  | { type: 'SwitchLoader'; target_version: string };

/**
 * Launch an instance with an optional recovery action performed before the
 * launch. The recovery action runs in the same backend operation; if it fails
 * the launch is aborted. Returns the PID on success.
 */
export const launchInstanceWithRecovery = (
  instanceId: string,
  action: LaunchRecoveryAction,
  allowHealthBlockers = false,
  healthScanToken?: string,
) => invoke<number>('launch_instance_with_recovery', { instanceId, action, allowHealthBlockers, healthScanToken: healthScanToken ?? null });

export const killProcess = (pid: number) =>
  invoke<void>('kill_process', { pid });

export interface UpdateInfo {
  filename: string;
  mod_jar_id: string;
  current_version: string;
  latest_version: string;
  target_version: string;
  source: string;
}

export const checkInstanceUpdates = (instanceId: string) =>
  invoke<UpdateInfo[]>('check_instance_updates', { instanceId });

export interface VersionChangelog {
  item_id: string;
  version: string;
  /** Markdown. Render with react-markdown; never dangerouslySetInnerHTML. */
  changelog: string;
  published_at: string | null;
  source: string;
}

/**
 * Changelogs for every release between the installed version and the update.
 * Read from the signed registry — offline, and empty when the item simply has
 * no upstream changelogs rather than on error.
 */
export const getUpdateChangelogs = (itemId: string, fromVersion: string, toVersion: string) =>
  invoke<VersionChangelog[]>('get_update_changelogs', { itemId, fromVersion, toVersion });

/** Pin or unpin an installed entry against updates. Resolves false if nothing matched. */
export const setModUpdatePinned = (instanceId: string, filename: string, pinned: boolean) =>
  invoke<boolean>('set_mod_update_pinned', { instanceId, filename, pinned });

/** Instant, offline read of the last persisted update check (no network). */
export const getCachedInstanceUpdates = (instanceId: string) =>
  invoke<UpdateInfo[] | null>('get_cached_instance_updates', { instanceId });

export interface CachedInstanceUpdates {
  instance_id: string;
  updates: UpdateInfo[];
  checked_at: string;
}

/** Instant hydration of all cached update rows (no network). */
export const getCachedAllUpdates = () =>
  invoke<CachedInstanceUpdates[]>('get_cached_all_updates');

/** Invalidate the cached row after a successful install (view invalidation). */
export const clearCachedInstanceUpdates = (instanceId: string) =>
  invoke<void>('clear_cached_instance_updates', { instanceId });

export interface RunningProcess {
  instance_id: string;
  pid: number;
  session_id: number;
}

/**
 * Every tracked direct-launch process. Several instances can run at once, and
 * the same instance can be launched more than once, so this is a list.
 */
export const queryLaunchState = () =>
  invoke<RunningProcess[]>('query_launch_state');

export const getLkgMarker = (instanceId: string) =>
  invoke<Record<string, unknown> | null>('get_lkg_marker', { instanceId });

export const exportLockfile = (instanceId: string) =>
  invoke<Record<string, unknown>>('export_lockfile', { instanceId });

export interface SnapshotDiffEntry {
  path: string;
  oldSha256: string | null;
  newSha256: string | null;
  oldSize: number | null;
  newSize: number | null;
}

export interface SnapshotDiff {
  fromId: string | null;
  toId: string | null;
  added: SnapshotDiffEntry[];
  removed: SnapshotDiffEntry[];
  modified: SnapshotDiffEntry[];
  unchangedCount: number;
  totalFilesA: number;
  totalFilesB: number;
}

export const detectDrift = (instanceId: string, snapshotId: string) =>
  invoke<SnapshotDiff>('detect_drift', { instanceId, snapshotId });

export interface LockfileDriftDifference {
  path: string;
  kind: 'added' | 'removed' | 'modified' | 'enabled' | 'disabled' | 'config-modified';
  expectedSha256: string | null;
  actualSha256: string | null;
}

export interface LockfileDriftReport {
  status: 'in-sync' | 'drifted';
  differences: LockfileDriftDifference[];
}

export const verifyLockfile = (instanceId: string, lockfileJson: string) =>
  invoke<LockfileDriftReport>('verify_lockfile', { instanceId, lockfileJson });

export const repairLockfile = (instanceId: string, lockfileJson: string) =>
  invoke<import('./installFlow').InstallOutcome>('repair_lockfile', { instanceId, lockfileJson });

export const importLockfile = (lockfileJson: string) =>
  invoke<string>('import_lockfile', { lockfileJson });

export type HealthScore = 'green' | 'yellow' | 'red';

/** Structured loader-compatibility evidence on a health finding (Rust
 * `health::LoaderCompatibilityIssue`). The UI renders this payload directly
 * and never parses the finding's human-readable `message` to make decisions. */
export interface LoaderCompatibilityIssue {
  loader: string;
  current_version: string | null;
  recommended_version: string | null;
  compatible_versions: string[];
  indeterminate_versions?: string[];
  requirements: LoaderRequirementIssue[];
  conflicts: LoaderConflict[];
}

/** Structured evidence for one loader requirement evaluation (Rust
 * `health::LoaderRequirementIssue`). */
export interface LoaderRequirementIssue {
  declaring_mod_id: string | null;
  declaring_mod_ids?: string[];
  target_id: string;
  version_ranges: string[];
  importance: DependencyImportance;
  candidate_version: string | null;
  verdict: RequirementVerdict;
}

/** Serialized `dependency_ops::DependencyImportance`. */
export type DependencyImportance = 'required' | 'recommended' | 'suggested';

/** Serialized `dependency_ops::DependencySource`. */
export type DependencySource =
  | 'fabric_depends'
  | 'fabric_recommends'
  | 'fabric_suggests'
  | 'quilt_depends'
  | 'forge_dependency'
  | 'neoforge_dependency'
  | 'forge_language_loader'
  | 'neoforge_language_loader';

/** Serialized `dependency_ops::VersionGrammar`. */
export type VersionGrammar = 'fabric' | 'maven';

/** Serialized `dependency_ops::DependencyDecl` as carried inside loader
 * requirement evidence. */
export interface DependencyDecl {
  declaring_mod_id: string | null;
  target_id: string;
  version_ranges: string[];
  importance: DependencyImportance;
  grammar: VersionGrammar;
  source: DependencySource;
}

/** Serialized `loader_compatibility::RequirementVerdict`. */
export type RequirementVerdict =
  | 'satisfied'
  | 'unsatisfied'
  | { unsupported: { reason: string } };

/** Serialized `loader_compatibility::LoaderConflict`. */
export interface LoaderConflict {
  declaring_mod_id: string | null;
  target_id: string;
  version_ranges: string[];
  with_declaring_mod_id: string | null;
  with_target_id: string;
  with_version_ranges: string[];
  message: string;
}

export interface HealthWarning {
  kind: string;
  mod_id: string | null;
  /** Filename on disk when this finding references an installed mod, null otherwise. */
  filename: string | null;
  message: string;
  suggested_action: string | null;
  /** Present when this warning is a loader requirement finding. */
  loader_compatibility?: LoaderCompatibilityIssue | null;
}

export interface HealthBlocker {
  kind: string;
  mod_id: string | null;
  /** Filename on disk when this finding references an installed mod, null otherwise. */
  filename: string | null;
  message: string;
  suggested_action: string | null;
  /** Present when this blocker is a loader requirement finding. */
  loader_compatibility?: LoaderCompatibilityIssue | null;
}

export interface HealthRecommendation {
  kind: string;
  mod_id: string | null;
  source_filename: string | null;
  message: string;
  suggested_action: string | null;
}

export interface HealthReport {
  score: HealthScore;
  warnings: HealthWarning[];
  blockers: HealthBlocker[];
  recommendations: HealthRecommendation[];
  scan_token: string;
}

export const checkInstanceHealth = (instanceId: string) =>
  invoke<HealthReport>('check_instance_health', { instanceId });

/** One background scan result. A per-instance failure is isolated so other
 * cards retain their current health status. */
export interface InstanceHealthScanResult {
  instance_id: string;
  report: HealthReport | null;
  error: string | null;
}

/** Periodically used by the app shell to scan every local instance. */
export const checkAllInstanceHealth = () =>
  invoke<InstanceHealthScanResult[]>('check_all_instance_health');

export const listLoaderVersions = (loader: string, mcVersion: string) =>
  invoke<LoaderVersionSummary[]>('list_loader_versions', {
    loader,
    mcVersion,
  });

// --- Loader version switching (Work Package 7) ---

/** Serialized `loader_manifests::LoaderReleaseChannel`. */
export type LoaderReleaseChannel = 'stable' | 'prerelease';

/** Serialized `loader_manifests::LoaderCapabilities`. */
export interface LoaderCapabilities {
  distribution_id: string;
  distribution_version: string;
  provided_versions: Record<string, string>;
}

/** Serialized `loader_compatibility::CurrentLoaderStatus`. */
export type CurrentLoaderStatus =
  | 'compatible'
  | 'incompatible'
  | 'indeterminate'
  | 'no_compatible_candidates';

/** Serialized `loader_compatibility::LoaderRequirementResult`. */
export interface LoaderRequirementResult {
  declaration: DependencyDecl;
  declaring_mod_ids?: string[];
  capability: string;
  kind: 'framework' | 'language_loader';
  candidate_provided_version: string | null;
  verdict: RequirementVerdict;
}

/** A signed catalog candidate that satisfies every hard requirement
 * (serialized `loader_compatibility::CompatibleLoaderCandidate`). */
export interface CompatibleLoaderCandidate {
  loader_version: string;
  release_channel: LoaderReleaseChannel;
  recommendation_rank: number | null;
  capabilities: LoaderCapabilities;
  requirement_results: LoaderRequirementResult[];
}

/** Full compatibility report (serialized
 * `loader_compatibility::LoaderCompatibilityReport`). */
export interface LoaderCompatibilityReport {
  current_status: CurrentLoaderStatus;
  requirements: LoaderRequirementResult[];
  compatible_versions: CompatibleLoaderCandidate[];
  indeterminate_versions?: CompatibleLoaderCandidate[];
  recommended_version: CompatibleLoaderCandidate | null;
  conflicts: LoaderConflict[];
}

/** Preview of a loader version switch (serialized
 * `loader_service::LoaderChangePlan`). Planning performs no mutation. */
export interface LoaderChangePlan {
  instance_id: string;
  loader: string;
  minecraft_version: string;
  current_loader_version: string;
  recommended_loader_version: string | null;
  current_report: LoaderCompatibilityReport;
}

/** Committed result of a loader version switch (serialized
 * `loader_service::LoaderChangeResult`). */
export interface LoaderChangeResult {
  instance_id: string;
  previous_loader_version: string;
  loader_version: string;
  health: HealthReport;
}

/** Plan a loader version switch without mutating anything. */
export const planLoaderChange = (instanceId: string) =>
  invoke<LoaderChangePlan>('plan_loader_change', { instanceId });

/** Commit a loader version switch to an explicit signed catalog target. */
export const changeLoaderVersion = (
  instanceId: string,
  targetVersion: string,
  allowIndeterminate = false,
) => invoke<LoaderChangeResult>('change_loader_version', {
  instanceId,
  targetVersion,
  allowIndeterminate,
});
export const listManifestLoaders = () =>
  invoke<string[]>('list_manifest_loaders');
export const listManifestMcVersions = (loader?: string) =>
  invoke<string[]>('list_manifest_mc_versions', { loader });
export const forYouItems = (
  mcVersion?: string,
  loader?: string,
  limit?: number,
  modrinthCategories?: string[],
  query?: string,
) =>
  invoke<RegistryItem[]>('for_you_items', {
    mcVersion,
    loader,
    limit,
    modrinthCategories,
    query,
  });

export const browseItems = (
  contentType?: string,
  category?: string,
  sort?: SortOption,
  mcVersion?: string,
  loader?: string,
  limit?: number,
) =>
  invoke<RegistryItem[]>('browse_items', {
    contentType,
    category,
    sort,
    mcVersion,
    loader,
    limit,
  });
export const getRegistryItem = (itemId: string) =>
  invoke<RegistryItem | null>('get_registry_item', { itemId });
export const listCategories = () => invoke<CategoryInfo[]>('list_categories');

// --- Governance / Transparency Log ---

/**
 * AuditLogEntry — mirrors the Rust `AuditLogEntry` struct in
 * desktop/src-tauri/src/registry.rs. Keep these two definitions in sync:
 * adding/removing/renaming a field on the Rust struct requires the same change
 * here, or the value will be silently dropped at the IPC boundary.
 * TODO: replace this hand-mirror with generated types (e.g. ts-rs) once a
 * codegen step is wired into the build.
 */
export interface AuditLogEntry {
  id: number;
  timestamp: string;
  action: string;
  details: string | null;
}

export const listAuditLog = (limit?: number) =>
  invoke<AuditLogEntry[]>('list_audit_log', { limit });
export const checkRegistryUpdate = (force?: boolean) =>
  invoke<RegistryStatus>('check_registry_update', { force });
export const getRegistryStatus = () => invoke<RegistryStatus>('get_registry_status');
export const extractOverrides = (zipPath: string, instanceId: string) =>
  invoke<ExtractionResult>('extract_overrides', { zipPath, instanceId });
export const getSetting = (key: string) =>
  invoke<unknown | null>('get_setting', { key });
export const setSetting = (key: string, value: unknown) =>
  invoke<void>('set_setting', { key, value });

export interface DeviceFlowResponse {
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export interface GithubProfile {
  login: string;
  avatar_url: string;
}

export const githubLogin = () => invoke<DeviceFlowResponse>('github_login');
export const githubLoginPoll = (deviceCode: string, interval: number) =>
  invoke<boolean>('github_login_poll', { deviceCode, interval });
export const githubLogout = () => invoke<void>('github_logout');
export const getAuthStatus = () => invoke<boolean>('get_auth_status');
export const getGithubProfile = () =>
  invoke<GithubProfile | null>('get_github_profile');

export interface CrashReportInfo {
  filename: string;
  modified_at: string;
  size_bytes: number;
}

export interface CrashTriageResult {
  matched: boolean;
  signature_name: string | null;
  solution_markdown: string | null;
  action_button_json: string | null;
}

export type EvidenceSourceKind =
  | 'CrashReport'
  | 'LatestLog'
  | 'DebugLog'
  | 'JvmFatalErrorLog'
  | 'UserAdded'
  | 'UserPasted';

export interface CrashEvidenceSource {
  meta: {
    basename: string;
    kind: EvidenceSourceKind;
    size_bytes: number;
    truncated: boolean;
    stale: boolean;
    supplementary: boolean;
    modified_at: string | null;
    line_count: number;
  };
  text: string;
}

export interface CrashInvestigation {
  evidence: {
    sources: CrashEvidenceSource[];
    primary_index: number;
    aggregate_bytes: number;
    any_truncated: boolean;
    any_stale: boolean;
    failure_category: 'CrashReport' | 'Oom' | 'JvmFatal' | 'NoEvidence';
  };
  fingerprint: CrashFingerprint | null;
  triage: CrashTriageResult;
  suspects: SuspectScore[];
  failure_category: 'CrashReport' | 'Oom' | 'JvmFatal' | 'NoEvidence';
}

export const checkInstanceCrash = (instanceId: string) =>
  invoke<CrashReportInfo | null>('check_instance_crash', { instanceId });
export const triageCrashReport = (instanceId: string, filename: string) =>
  invoke<CrashTriageResult>('triage_crash_report', { instanceId, filename });
export const listCrashReports = (instanceId: string) =>
  invoke<CrashReportInfo[]>('list_crash_reports_cmd', { instanceId });
export const readCrashLog = (instanceId: string, filename: string) =>
  invoke<string>('read_crash_log_cmd', { instanceId, filename });
export const investigateInstanceEvidence = (instanceId: string) =>
  invoke<CrashInvestigation>('investigate_instance_evidence', { instanceId });
export const pickAndInvestigateCrashEvidence = (instanceId: string) =>
  invoke<CrashInvestigation | null>('pick_and_investigate_crash_evidence', { instanceId });

export interface ModVersionCandidate {
  version: string;
  filename: string;
  download_url: string;
  mc_version: string | null;
  loader: string | null;
  release_date: string | null;
  is_compatible: boolean;
  sha1?: string | null;
  version_compat?: string;
  is_prerelease?: boolean;
  /// Which of the item's download sources produced this candidate. Set by the
  /// backend resolver; absent on candidates from an older backend.
  source_strategy?: string | null;
  source_identifier?: string | null;
}

export interface ModVersionPage {
  items: ModVersionCandidate[];
  hasMore: boolean;
  total: number;
}

export const listModVersions = (instanceId: string | null, itemId: string) =>
  invoke<ModVersionPage>('list_mod_versions', { instanceId, itemId });

export const listModVersionsLoadMore = (instanceId: string | null, itemId: string, page: number) =>
  invoke<ModVersionPage>('list_mod_versions_load_more', { instanceId, itemId, page });

/// Quick compat probe for the browse page — returns
/// `"compatible"`, `"major_match"`, or `""` (incompatible).
export const checkModCompat = (instanceId: string, itemId: string) =>
  invoke<string>('check_mod_compat', { instanceId, itemId });

export const batchCheckCompat = (instanceId: string, itemIds: string[]) =>
  invoke<Record<string, string>>('batch_check_compat', { instanceId, itemIds });

export const disableInstanceMod = (instanceId: string, filename: string) =>
  invoke<void>('disable_instance_mod', { instanceId, filename });
export const enableInstanceMod = (instanceId: string, filename: string) =>
  invoke<void>('enable_instance_mod', { instanceId, filename });

export const exportInstancePack = (instanceId: string, format: 'json' | 'mrpack') =>
  invoke<string>('export_instance_pack', { instanceId, format });

export const pickOpenFile = (title: string, extensions: string[]) =>
  invoke<string | null>('pick_open_file', { title, extensions });
export const setCustomInstanceIcon = (instanceId: string, sourcePath: string) =>
  invoke<string>('set_custom_instance_icon', { instanceId, sourcePath });
export const setCustomModIcon = (instanceId: string, filename: string, sourcePath: string) =>
  invoke<string>('set_custom_mod_icon', { instanceId, filename, sourcePath });
export const getCustomIcon = (instanceId: string, target: 'instance' | 'mod', filename?: string) =>
  invoke<string | null>('get_custom_icon', { instanceId, target, filename: filename ?? null });

export type LauncherKind = 'prism' | 'curse_forge' | 'modrinth';
export type CandidateStatus = 'ready' | 'needs_review' | { unsupported: { reasons: string[] } };

export interface LoaderTuple {
  loader: string;
  loader_version: string;
  minecraft_version: string;
}

export interface LaunchSettingsPreview {
  memory_mb: number | null;
  java_path: string | null;
  jvm_args: string[];
}

export interface ContentInventory {
  payload_root: string;
  total_files: number;
  total_bytes: number;
  has_mods: boolean;
  has_resourcepacks: boolean;
  has_shaderpacks: boolean;
  has_datapacks: boolean;
  has_saves: boolean;
}

export interface LauncherImportCandidate {
  source_key: string;
  launcher: LauncherKind;
  launcher_installation_key: string;
  display_name: string;
  icon_path: string | null;
  payload_root: string;
  inventory: ContentInventory;
  loader_tuple: LoaderTuple | null;
  last_played: string | null;
  launch_strategy: 'normal' | 'delegated';
  settings_preview: LaunchSettingsPreview;
  status: CandidateStatus;
  warnings: string[];
}

export interface DetectedLauncherImport {
  installation_key: string;
  kind: LauncherKind;
  display_name: string;
  config_root: string;
  instances_dir: string;
  instance_count: number;
  detection_warnings: string[];
}

export interface LauncherDiscovery {
  launcher: DetectedLauncherImport | null;
  candidates: LauncherImportCandidate[];
}

export interface LauncherImportDiscovery {
  prism: LauncherDiscovery;
  curseforge: LauncherDiscovery;
  modrinth: LauncherDiscovery;
}

export interface ImportSelection {
  source_key: string;
  launcher_kind: LauncherKind;
  installation_key: string;
  destination_name: string | null;
  preserve_settings: boolean;
}

export interface LauncherImportItemPlan {
  fingerprint: string;
  destination_id: string;
  destination_name: string;
  action: 'new' | 'update' | 'unchanged';
  source_key: string;
  launcher_kind: LauncherKind;
  installation_key: string;
  source_path: string;
  loader_tuple: LoaderTuple | null;
  total_bytes: number;
  total_files: number;
  preserve_settings: boolean;
  sanitized_settings: LaunchSettingsPreview;
  existing_import: unknown | null;
  blockers: string[];
  warnings: string[];
}

export interface LauncherImportPlan {
  batch_fingerprint: string;
  items: LauncherImportItemPlan[];
  peak_bytes: number;
  total_files: number;
  batch_blockers: string[];
}

export type LauncherImportOutcome =
  | { status: 'imported'; instance_id: string; warnings: string[] }
  | { status: 'updated'; instance_id: string; warnings: string[] }
  | { status: 'skipped'; reason: string }
  | { status: 'failed'; error: string; warnings: string[] }
  | { status: 'cancelled'; reason: string };

export interface LauncherImportBatchResult {
  outcomes: LauncherImportOutcome[];
}

export const pickDirectory = (title: string) =>
  invoke<string | null>('pick_directory', { title });
export const discoverLauncherImports = (customRoot?: string | null) =>
  invoke<LauncherImportDiscovery>('discover_launcher_imports', { customRoot: customRoot ?? null });
export const planLauncherImports = (selections: ImportSelection[]) =>
  invoke<LauncherImportPlan>('plan_launcher_imports', { selections });
export const executeLauncherImports = (plan: LauncherImportPlan) =>
  invoke<LauncherImportBatchResult>('execute_launcher_imports', { plan });

export const importInstancePack = (sourcePath: string) =>
  invoke<string>('import_instance_pack', { sourcePath });
export const importModrinthPackByUrl = (downloadUrl: string, packIconUrl?: string | null) =>
  invoke<string>('import_modrinth_pack_by_url', {
    downloadUrl,
    packIconUrl: packIconUrl ?? null,
  });

// --- Raw (uncurated) Modrinth integration (§6.3) ---

export interface ModrinthSearchResult {
  project_id: string;
  slug: string;
  title: string;
  description: string;
  icon_url: string | null;
  author: string;
  categories: string[];
  downloads: number;
  follows: number;
  project_type: string;
  date_created: string | null;
  date_modified: string | null;
  versions: string[];
  license: string | null;
  gallery: string[];
  featured_gallery: string | null;
}

export type ModrinthSort = 'relevance' | 'follows' | 'newest' | 'updated';

export interface ModrinthSearchParams {
  query?: string;
  categories?: string[];
  loaders?: string[];
  game_versions?: string[];
  project_type?: string;
  sort?: ModrinthSort;
  offset?: number;
  limit?: number;
}

export interface ModrinthSearchPage {
  results: ModrinthSearchResult[];
  total_hits: number;
  offset: number;
  limit: number;
}

export interface ModrinthCategoryInfo {
  name: string;
  project_type: string;
  header: string;
}

export interface ModrinthLoaderInfo {
  name: string;
  supported_project_types: string[];
}

export interface ModrinthGameVersionInfo {
  version: string;
  version_type: string;
  date: string;
  major: boolean;
}

export interface RawModrinthVersionCandidate {
  version: string;
  version_id: string;
  name: string;
  filename: string;
  download_url: string;
  sha1: string | null;
  mc_versions: string[];
  loaders: string[];
  release_date: string | null;
  primary: boolean;
  changelog: string | null;
  is_prerelease?: boolean;
  version_type?: string | null;
}

export const isModrinthEnabled = () => invoke<boolean>('is_modrinth_enabled');
export const searchModrinth = (params: ModrinthSearchParams) =>
  invoke<ModrinthSearchPage>('search_modrinth', { params });
export const listModrinthCategories = () =>
  invoke<ModrinthCategoryInfo[]>('list_modrinth_categories');
export const listModrinthLoaders = () =>
  invoke<ModrinthLoaderInfo[]>('list_modrinth_loaders');
export const listModrinthGameVersions = () =>
  invoke<ModrinthGameVersionInfo[]>('list_modrinth_game_versions');
export const listRawModrinthVersions = (instanceId: string | null, projectId: string, projectType?: string) =>
  invoke<RawModrinthVersionCandidate[]>('list_raw_modrinth_versions', {
    instanceId,
    projectId,
    projectType: projectType ?? null,
  });
export interface ModrinthProjectFull {
    id: string;
    title: string;
    description: string;
    body: string | null;
    icon_url: string | null;
    project_type: string;
    page_url: string | null;
    license_id: string | null;
    source_updated_at: string | null;
    gallery_urls: string[];
    /** Modrinth category slugs, lowercase as published. */
    categories: string[];
    downloads: number;
    followers: number;
}

export const fetchModrinthProject = (projectId: string) =>
  invoke<ModrinthProjectFull>('fetch_modrinth_project', { projectId });

// --- Technic source (consent tiers S/Z) ---

export type TechnicTier = 'solder' | 'zip';

export interface TechnicSearchResult {
  slug: string;
  title: string;
  description: string;
  installs: number;
  likes: number;
  author: string | null;
  page_url: string;
  icon_url: string | null;
  tags: string[];
  tier: TechnicTier;
}

export interface TechnicPackDetail {
  slug: string;
  title: string;
  description: string;
  installs: number;
  likes: number;
  author: string | null;
  solder: string | null;
  recommended_build: string | null;
  minecraft: string | null;
  website: string | null;
  page_url: string;
  icon_url: string | null;
  tags: string[];
  download_url: string | null;
  tier: TechnicTier;
  permitted: boolean;
}

export interface ImportResult {
  instance_id: string;
  name: string;
  minecraft_version: string;
  loader: string;
  loader_version: string;
  imported_mods: number;
  linked_saves: boolean;
}

export const technicSearch = (query: string, limit?: number) =>
  invoke<TechnicSearchResult[]>('technic_search', { query, limit });
export const technicPackDetail = (slug: string) =>
  invoke<TechnicPackDetail>('technic_pack_detail', { slug });
export const installTechnicSolderPack = (slug: string, solder: string, build: string) =>
  invoke<ImportResult>('install_technic_solder_pack', { slug, solder, build });
export const installTechnicZipPack = (
  name: string,
  downloadUrl: string,
  sha256: string | null,
  minecraftVersion: string,
  loader: string,
  loaderVersion: string,
) =>
  invoke<ImportResult>('install_technic_zip_pack', {
    name,
    downloadUrl,
    sha256,
    minecraftVersion,
    loader,
    loaderVersion,
  });

// --- Phase 7: Curated annotation overlay for registry-backed items ---

export interface CuratedAnnotation {
  id: string;
  name: string;
  curator_note: string | null;
  net_score: number | null;
  is_immune: boolean;
  base_categories: string[];
}

// `itemId` must be a registry item id (registry_items.id). The backend query
// resolves curated features by registry id and pulls the real curator note
// from curator_reviews.curator_note; passing a Modrinth project id here will
// not match a curated entry. Callers must gate on isRegistryBacked first.
export const getCuratedAnnotation = (itemId: string) =>
  invoke<CuratedAnnotation | null>('get_curated_annotation', { itemId });

// --- Governance / Triage ---

export interface GovernanceConfig {
  repository: string;
  environment: 'production' | 'sandbox';
  github_app_slug: string | null;
  development_registry: boolean;
}

export interface GovernanceSummary {
  item_id: string;
  vote_issue_number: number | null;
  vote_issue_url: string | null;
  raw_upvotes: number;
  raw_downvotes: number;
  counted_upvotes: number;
  counted_downvotes: number;
  quarantined_upvotes: number;
  quarantined_downvotes: number;
  conflicted_users: number;
  status_reason: string | null;
  compiled_at: string;
}

export type ItemVote = 'upvote' | 'downvote';

export interface ItemVoteState {
  vote: ItemVote | null;
  conflicted: boolean;
}

export interface GovernanceEvent {
  event_id: string;
  item_id: string | null;
  event_type: string;
  status: string;
  detected_at: string;
  affected_reactions: number;
  details_json: string | null;
}

export interface DiagnosticCheck {
  id: string;
  status: 'pass' | 'warning' | 'fail';
  message: string;
}

export interface UnderReviewItem {
  id: string;
  name: string;
  content_type: string;
  icon_url: string | null;
  net_score: number;
}

export interface ModReview {
  author: string | null;
  text: string;
  issue_number: number;
  created_at: string | null;
  item_version?: string | null;
  minecraft_version?: string | null;
  loader?: string | null;
  relationship?: string | null;
  focus?: string[];
  evidence?: string | null;
  limitations?: string | null;
  issue_url?: string | null;
}

export interface TriagePoll {
  discussion_url: string | null;
  keep_votes: number;
  remove_votes: number;
}

export const getGovernanceConfig = () =>
  invoke<GovernanceConfig | null>('get_governance_config');

export const getGovernanceSummary = (itemId: string) =>
  invoke<GovernanceSummary | null>('get_governance_summary', { itemId });

export const getItemVote = (itemId: string) =>
  invoke<ItemVoteState>('get_item_vote', { itemId });

export const setItemVote = (itemId: string, vote: ItemVote | null) =>
  invoke<ItemVoteState>('set_item_vote', { itemId, vote });

export const listGovernanceEvents = (itemId: string | null) =>
  invoke<GovernanceEvent[]>('list_governance_events', {
    itemId,
  });

export const runGovernanceDiagnostics = () =>
  invoke<DiagnosticCheck[]>('run_governance_diagnostics');

export const listUnderReviewItems = () =>
  invoke<UnderReviewItem[]>('list_under_review_items');

export const listRecentResolutions = (limit?: number) =>
  invoke<AuditLogEntry[]>('list_recent_resolutions', { limit });

export const listModReviews = (itemId: string) =>
  invoke<ModReview[]>('list_mod_reviews', { itemId });

export const fetchTriagePoll = (modId: string) =>
  invoke<TriagePoll>('fetch_triage_poll', { modId });

// --- Crash Investigation (guided isolation) ---

export interface CrashFingerprint {
  exception_class: string;
  top_frames: string[];
}

export interface SuspectScore {
  mod_id: string;
  filename: string;
  total_score: number;
  breakdown: Record<string, unknown>;
  is_dependent_of: string | null;
}

export type SuggestedAction =
  | { kind: 'GuidedDisable'; next_suspect: SuspectScore }
  | { kind: 'ConfidenceAutoDisable'; mod_id: string; filename: string }
  | { kind: 'ShowTriageBanner'; mod_id: string }
  | { kind: 'NoSuspects' };

export interface InvestigationResult {
  fingerprint: CrashFingerprint | null;
  signature_name: string | null;
  suspects: SuspectScore[];
  suggested_action: SuggestedAction;
  ruled_out: string[];
}

export const investigateCrash = (instanceId: string, filename?: string) =>
  invoke<InvestigationResult>('investigate_crash', { instanceId, filename });

export const investigateManual = (instanceId: string, logText: string) =>
  invoke<InvestigationResult>('investigate_manual', { instanceId, logText });

export const disableModForTest = (instanceId: string, filename: string) =>
  invoke<void>('disable_mod_for_test', { instanceId, filename });

export const enableModForTest = (instanceId: string, filename: string) =>
  invoke<void>('enable_mod_for_test', { instanceId, filename });

export const confirmCrashFix = (fingerprint: CrashFingerprint, modId: string) =>
  invoke<void>('confirm_crash_fix', { fingerprint, modId });

export const reportStillCrashing = (
  instanceId: string,
  fingerprint: CrashFingerprint,
  ruledOutModId: string,
  crashLogText: string,
) =>
  invoke<InvestigationResult>('report_still_crashing', {
    instanceId,
    fingerprint,
    ruledOutModId,
    crashLogText,
  });

// --- Dependency Plans (PREVIEW) ---

/**
 * These MUST stay lowercase: `dependency_ops::Requirement` and `DepSource` derive
 * `#[serde(rename_all = "kebab-case")]`, so the wire values are `'required'` /
 * `'optional'` and `'jar'` / `'manifest'`.
 *
 * They were previously declared capitalised, which typechecked fine and failed
 * silently at runtime — `'required' === 'Required'` is just false, so required
 * dependencies were never detected as required and never pre-ticked. Capitalise
 * for DISPLAY at the point of rendering, never in the comparison.
 */
export type Requirement = 'required' | 'optional';

export type DepSource = 'jar' | 'manifest';

export interface DependentInfo {
  mod_id: string;
  filename: string;
  requirement: Requirement;
  source: DepSource;
}

export interface DepCandidate {
  mod_jar_id: string;
  requirement: Requirement;
  source: DepSource;
}

export interface DepConflict {
  mod_jar_id: string;
  jar_requirement: Requirement | null;
  manifest_requirement: Requirement | null;
}

export interface InstallPlan {
  missing_required: DepCandidate[];
  missing_optional: DepCandidate[];
  conflicts: DepConflict[];
}

export interface RemovalPlan {
  dependents: DependentInfo[];
}

export interface DisablePlan {
  dependents: DependentInfo[];
}

/** Serialized `dependency_ops::DependencyEdge`. */
export interface DependencyEdge {
  from_filename: string;
  to_filename: string;
  requirement: Requirement;
}

/** Serialized `dependency_ops::OrphanedDependency`. */
export interface OrphanedDependency {
  filename: string;
  mod_jar_id: string | null;
  content_type: string;
}

/** Serialized `dependency_ops::PresenceExplanation`. */
export interface PresenceExplanation {
  filename: string;
  installed_as_dependency: boolean;
  pack_managed: boolean;
  dependents: DependentInfo[];
  /** Shortest chains from a user-installed mod down to this item, root first. */
  root_paths: string[][];
  orphaned: boolean;
}

/**
 * Mods that were installed only as dependencies and that nothing needs any
 * more. Read this after a removal — the answer is always about the manifest as
 * it stands right now.
 */
export const getOrphanedDependencies = (instanceId: string) =>
  invoke<OrphanedDependency[]>('get_orphaned_dependencies', { instanceId });

/** "Why is this mod here?" — traces one item back to the mods that need it. */
export const explainModPresence = (instanceId: string, filename: string) =>
  invoke<PresenceExplanation | null>('explain_mod_presence', { instanceId, filename });

/** Every dependency edge between installed content, in one read. */
export const getDependencyGraph = (instanceId: string) =>
  invoke<DependencyEdge[]>('get_dependency_graph', { instanceId });

export const getDisablePlan = (instanceId: string, filename: string) =>
  invoke<DisablePlan>('get_disable_plan', { instanceId, filename });

export const getRemovalPlan = (instanceId: string, filename: string) =>
  invoke<RemovalPlan>('get_removal_plan', { instanceId, filename });

export const getInstallPlan = (instanceId: string, itemId: string, jarPath: string) =>
  invoke<InstallPlan>('get_install_plan', { instanceId, itemId, jarPath });

export const enableModWithAutoDeps = (instanceId: string, filename: string) =>
  invoke<string[]>('enable_mod_with_auto_deps', { instanceId, filename });

// --- MCP Server Lifecycle ---

export interface McpStatus {
  running: boolean;
  url: string | null;
}

export const startMcpServer = () => invoke<McpStatus>('start_mcp_server');
export const stopMcpServer = () => invoke<void>('stop_mcp_server');
export const getMcpStatus = () => invoke<McpStatus>('get_mcp_status');
export const getMcpSkillContent = () => invoke<string>('get_mcp_skill_content');
export const setMcpApproval = (toolName: string, instanceId: string, state: string) =>
  invoke<void>('set_mcp_approval', { toolName, instanceId, state });

// --- MCP Token Management ---

export interface McpTokenData {
  token: string;
  config_snippet: string;
}

export const getMCPToken = () => invoke<McpTokenData>('get_mcp_token');
export const regenerateMCPToken = () => invoke<McpTokenData>('regenerate_mcp_token');

// --- AI Assistant (GitHub Models) ---

export interface ChatMessage {
  role: string;
  content: string;
}

export interface ChatResponse {
  content: string;
  model: string;
}

export interface AiContext {
  instance_id: string | null;
  crash_log: string | null;
  crash_signatures: string | null;
  suspects: string | null;
}

export const aiChat = (
  messages: ChatMessage[],
  context?: AiContext | null,
) =>
  invoke<ChatResponse>('ai_chat', {
    messages,
    context: context ?? null,
  });

export const getWindowsAccentColor = () =>
  invoke<string | null>('get_windows_accent_color');

// --- Launcher path helpers (B3) ---

/** Auto-detect the Mojang launcher executable path. */
export const detectMojangLauncher = () =>
  invoke<string>('detect_mojang_launcher');

/** Validate that a given launcher path exists and is a valid executable. */
export const testLauncherPath = (path: string) =>
  invoke<boolean>('test_launcher_path', { path });

// --- AI Copilot auth ---

export interface CopilotDeviceFlowResponse {
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export interface CopilotToken {
  access_token: string;
  copilot_token: string | null;
  endpoint: string;
  plan: string;
  username: string;
  stored_at: string;
}

export const copilotLogin = () =>
  invoke<CopilotDeviceFlowResponse>('copilot_login');

/** Try to use the existing governance GitHub token for Copilot, skipping the
 *  device flow if the token works and the user has a Copilot subscription. */
export const copilotTryGovernanceToken = () =>
  invoke<CopilotToken | null>('copilot_try_governance_token');

export const copilotLoginPoll = (deviceCode: string, interval: number) =>
  invoke<CopilotToken>('copilot_login_poll', { deviceCode, interval });

export const copilotStatus = () =>
  invoke<CopilotToken | null>('copilot_status');

export const copilotLogout = () =>
  invoke<void>('copilot_logout');

// ---------------------------------------------------------------------------
// Phase 5: MSA auth + GC architect
// ---------------------------------------------------------------------------

/** Safe account metadata returned to the UI. Authentication tokens never
 * cross the Tauri command boundary. */
export interface MsaAccountStatus {
  username: string;
  uuid: string;
  expires: string;
}

export type GcProfile = 'low_latency' | 'high_efficiency' | 'manual';

export interface GcResult {
  profile: GcProfile;
  jvm_args: string;
  heap_mb: number;
  total_ram_mb: number;
  cpu_threads: number;
  recommended: boolean;
}

export const msaLogin = () =>
  invoke<MsaAccountStatus>('msa_login');

export const msaGetStatus = () =>
  invoke<MsaAccountStatus | null>('msa_get_status');

export const msaRefresh = () =>
  invoke<MsaAccountStatus>('msa_refresh');

export const msaLogout = () =>
  invoke<void>('msa_logout');

export const computeGcArgs = (
  javaVersion: number,
  requestedHeapMb: number,
  manualArgs: string,
  gcMode: 'auto' | GcProfile,
  alwaysPreTouch = true,
) =>
  invoke<GcResult>('compute_gc_args', {
    javaVersion,
    requestedHeapMb,
    manualArgs,
    gcMode,
    alwaysPreTouch,
  });

// ---------------------------------------------------------------------------
// Phase 6: Instance lifecycle
// ---------------------------------------------------------------------------

export interface Snapshot {
  id: string;
  label: string | null;
  created_at: string;
  file_count: number;
  size_estimate: number;
  is_lkg: boolean;
  is_current_lkg: boolean;
  is_pre_restore: boolean;
}

export interface LoadoutProfile {
  name: string;
  enabled_mods: string[];
  created_at: string;
}

export interface ImportResult {
  instance_id: string;
  name: string;
  minecraft_version: string;
  loader: string;
  loader_version: string;
  imported_mods: number;
  linked_saves: boolean;
}

export interface DetectedLauncher {
  launcher_type: string;
  instances_dir: string;
  instance_count: number;
}

export interface ClonePrefs {
  copy_saves: boolean;
  copy_mods: boolean;
  copy_resource_packs: boolean;
  copy_shader_packs: boolean;
  copy_screenshots: boolean;
  copy_config: boolean;
  copy_servers: boolean;
  copy_options: boolean;
  use_hard_links: boolean;
  use_sym_links: boolean;
}

export interface ExportResult {
  total_mods: number;
  server_mods: number;
  removed_client_only: string[];
  server_jar_downloaded: boolean;
  start_scripts_created: boolean;
}

export const listSnapshots = (instanceId: string) =>
  invoke<Snapshot[]>('list_snapshots', { instanceId });

export const createSnapshot = (instanceId: string, label?: string) =>
  invoke<Snapshot>('create_snapshot', { instanceId, label });

export const restoreSnapshot = (instanceId: string, snapshotId: string) =>
  invoke<void>('restore_snapshot', { instanceId, snapshotId });

export const deleteSnapshot = (instanceId: string, snapshotId: string) =>
  invoke<void>('delete_snapshot', { instanceId, snapshotId });

/** Serialized `template_service::TemplateJvm`. Every field is optional; `null`
 *  means "leave the instance's own value alone". */
export interface TemplateJvm {
  java_path?: string | null;
  jvm_memory_mb?: number | null;
  jvm_memory_mode?: string | null;
  jvm_gc?: string | null;
  jvm_custom_args?: string | null;
  jvm_always_pre_touch?: boolean | null;
}

/** Serialized `template_service::TemplateFile`. */
export interface TemplateFile {
  relative_path: string;
  sha256: string;
  size: number;
}

/** Serialized `template_service::InstanceTemplate`. */
export interface InstanceTemplate {
  template_version: number;
  id: string;
  name: string;
  description: string | null;
  created_at: string;
  updated_at: string;
  jvm: TemplateJvm;
  files: TemplateFile[];
}

/** Serialized `template_service::CapturableFile`. */
export interface CapturableFile {
  relative_path: string;
  size: number;
  category: string;
  too_large: boolean;
}

/**
 * Write a snapshot out to a folder the user chose, returning the artifact path.
 * Point it at a folder Dropbox or OneDrive already syncs and backups go offsite
 * with no service behind them.
 */
export const exportBackup = (instanceId: string, snapshotId: string, exportDir: string) =>
  invoke<string>('export_backup', { instanceId, snapshotId, exportDir });

/** Read a backup artifact back into an instance. Fully validated before it
 *  touches the instance directory — the file is untrusted input. */
export const importBackup = (instanceId: string, artifactPath: string) =>
  invoke<Snapshot>('import_backup', { instanceId, artifactPath });

/** Apply a retention policy; resolves to the snapshot ids that were removed. */
export const applyBackupRetention = (
  instanceId: string,
  policy: { keepLast?: number | null; keepDays?: number | null },
) =>
  invoke<string[]>('apply_backup_retention', {
    instanceId,
    keepLast: policy.keepLast ?? null,
    keepDays: policy.keepDays ?? null,
  });

/** Serialized `migration_report::MigrationStatus`. */
export type MigrationStatus =
  | 'ready'
  | 'not_yet'
  | 'abandoned'
  | 'superseded'
  /** Could not be checked — a network failure, never a claim that it is dead. */
  | 'unknown'
  /** No Modrinth identity to check against; needs a human. */
  | 'unclassifiable';

/** Serialized `migration_report::MigrationVerdict`. */
export type MigrationVerdict = 'ready' | 'not_yet' | 'blocked' | 'unknown' | 'needs_review';

/** Serialized `migration_report::SuccessorInfo`. */
export interface SuccessorInfo {
  replacement_id: string;
  replacement_name: string | null;
  reason: string | null;
}

/** Serialized `migration_report::UnclassifiableReason`. */
export type UnclassifiableReason = 'manual' | 'curated_only' | 'other';

/** Serialized `migration_report::MigrationSummary`. */
export interface MigrationSummary {
  total: number;
  ready: number;
  not_yet: number;
  abandoned: number;
  superseded: number;
  unknown: number;
  unclassifiable: number;
}

/** Serialized `migration_report::ModMigrationEntry`. */
export interface ModMigrationEntry {
  filename: string;
  display_name: string;
  modrinth_id: string | null;
  registry_id: string | null;
  content_type: string;
  installed_version: string | null;
  status: MigrationStatus;
  /* The rest carry `skip_serializing_if` on the Rust side, so they are absent
     rather than null when they do not apply. */
  unclassifiable_reason?: UnclassifiableReason;
  last_updated?: string;
  has_target_build?: boolean;
  successor?: SuccessorInfo;
  /** Set only on `unknown` — why the check could not be made. */
  error_code?: string;
  error_message?: string;
}

/** Serialized `migration_report::MigrationReport`. */
export interface MigrationReport {
  instance_id: string;
  source_version: string;
  target_version: string;
  loader: string;
  summary: MigrationSummary;
  verdict: MigrationVerdict;
  mods: ModMigrationEntry[];
  warnings: string[];
}

/** Can this instance move to a newer Minecraft version, and what breaks?
 *  Read-only — running the migration is a separate, explicit step. */
export const getMigrationReport = (instanceId: string, targetVersion: string) =>
  invoke<MigrationReport>('get_migration_report', { instanceId, targetVersion });

/** Serialized `pack_merge::PlanActionKind`. */
export type PlanActionKind =
  | 'keep'
  | 'keep_user_added'
  | 'add'
  | 'remove'
  | 'update'
  | 'update_keep_disabled'
  | 'rename_update'
  | 'rename_update_keep_disabled';

/** Serialized `pack_merge::ConflictKind`. `no_baseline` means Agora has no
 *  record of what the pack originally installed, so a user edit cannot be told
 *  apart from a pack original — one question, not one per file. */
export type PackConflictKind =
  | 'both_modified'
  | 'added_vs_added'
  | 'modified_vs_removed'
  | 'removed_vs_modified'
  | 'ambiguous_disabled_pair'
  | 'duplicate_mod_id'
  | 'no_baseline';

/** Serialized `pack_merge::PlanAction` (snake_case). */
export interface PackPlanAction {
  key: string;
  logical_path: string;
  target_path: string;
  previous_path: string | null;
  kind: PlanActionKind;
  enabled: boolean;
  mod_id: string | null;
}

/** Serialized `pack_merge::PlanConflict` (snake_case). */
export interface PackPlanConflict {
  key: string;
  logical_path: string;
  kind: PackConflictKind;
  ours_path: string | null;
  theirs_path: string | null;
  message: string;
  mod_id: string | null;
}

/** Serialized `pack_merge::PackMergePlan` (snake_case). */
export interface PackMergePlan {
  actions: PackPlanAction[];
  conflicts: PackPlanConflict[];
  all_keys: string[];
  baseline_missing: boolean;
}

/** Serialized `pack_update::PackUpdatePreview` (camelCase). */
export interface PackUpdatePreview {
  plan: PackMergePlan;
  /** Paths whose mod-content decision is an estimate — the jar was not fetched. */
  unverified: string[];
  /** Unverified paths already byte-identical locally, so no download is needed. */
  converged: string[];
  filesNeedingDownload: number;
  downloadBytes: number;
  sizeUnknownCount: number;
  packName: string;
  packVersionId: string | null;
}

/** Serialized `pack_update::ConflictResolution`. */
export type ConflictResolution = 'keep_ours' | 'take_theirs';

/** Serialized `pack_update::PackUpdateOutcome` — tagged on `type`. */
export type PackUpdateOutcome =
  | { type: 'updated'; snapshotId: string; changed: number; kept: number; health: HealthReport | null }
  /** Kept, not rolled back — reverting would throw away the conflict answers
   *  the user just gave, and the pack may simply be broken. */
  | { type: 'health-blocked'; snapshotId: string; health: HealthReport }
  | { type: 'failed'; phase: string; error: string; rolledBack: boolean; snapshotId: string | null };

/** What updating to this pack file would do. Downloads nothing. */
export const previewPackUpdate = (instanceId: string, mrpackPath: string) =>
  invoke<PackUpdatePreview>('preview_pack_update', { instanceId, mrpackPath });

/** Apply a pack update. Every conflict in the preview must have an answer —
 *  core refuses otherwise rather than picking a side unasked. */
export const applyPackUpdate = (
  instanceId: string,
  mrpackPath: string,
  resolutions: Record<string, ConflictResolution>,
) => invoke<PackUpdateOutcome>('apply_pack_update', { instanceId, mrpackPath, resolutions });

/** Serialized `migration_report::TargetBuildInfo`. Snake_case — this type has
 *  no `rename_all`, unlike the version_migration types below. */
export interface TargetBuildInfo {
  version_id: string;
  version_number: string;
  filename: string;
  download_url: string;
  sha1?: string;
  sha512?: string;
  size?: number;
}

/** Serialized `version_migration::RejectionReason` (camelCase). */
export interface MigrationRejectionReason {
  code: string;
  message: string;
  filename?: string;
}

/** Serialized `version_migration::PlannedSwap` (camelCase). */
export interface PlannedSwap {
  oldFilename: string;
  contentType: string;
  oldEnabled: boolean;
  newFilename: string;
  target: TargetBuildInfo;
}

/** Serialized `version_migration::MigrationPlan` (camelCase). Read-only —
 *  building one mutates nothing. */
export interface MigrationPlan {
  instanceId: string;
  sourceVersion: string;
  targetVersion: string;
  loader: string;
  sourceLoaderVersion: string;
  targetLoaderVersion?: string | null;
  swaps: PlannedSwap[];
  /** Entries that will be left at their current version. Proceeding past these
   *  requires an explicit answer — see `runVersionMigration`. */
  blockers: MigrationRejectionReason[];
  warnings: string[];
  fingerprint: string;
  instanceStateHash: string;
  report: MigrationReport;
}

/** Serialized `version_migration::MigrationOutcome` — tagged on `type` in
 *  kebab-case, with camelCase fields. */
export type MigrationOutcome =
  | {
      type: 'migrated';
      instanceId: string;
      fromVersion: string;
      toVersion: string;
      loaderVersion?: string;
      replaced: string[];
      snapshotId: string;
      warnings: string[];
    }
  /** Refused before touching anything. */
  | { type: 'blocked'; reasons: MigrationRejectionReason[] }
  /** Mutated mid-way and verifiably restored. */
  | { type: 'rolled-back'; phase: string; error: string; snapshotId?: string }
  /** `rolledBack: false` means the instance may be mid-state and `snapshotId`
   *  is the recovery point. */
  | { type: 'failed'; phase: string; error: string; rolledBack: boolean; snapshotId?: string };

/** Plan a migration without performing it. */
export const planVersionMigration = (instanceId: string, targetVersion: string) =>
  invoke<MigrationPlan>('plan_version_migration', { instanceId, targetVersion });

/** Perform the migration. `acceptBlockers` must be the user's actual answer to
 *  the plan's blockers — passing it blindly turns "leave these mods behind"
 *  into a silent default. */
export const runVersionMigration = (
  instanceId: string,
  targetVersion: string,
  acceptBlockers: boolean,
) => invoke<MigrationOutcome>('run_version_migration', { instanceId, targetVersion, acceptBlockers });

/** Serialized `bisect::TrialOutcome`. */
export type BisectTrialOutcome = 'reproduced' | 'clean';

/** Serialized `bisect::BisectStep`. */
export interface BisectStep {
  enabled_suspects: string[];
  disabled_suspects: string[];
  outcome: BisectTrialOutcome | null;
}

/** Serialized `bisect::BisectStatus`. Internally tagged on `type`. */
export type BisectStatus =
  | { type: 'awaiting_trial' }
  | { type: 'culprit'; filename: string }
  /** Narrowed as far as the dependency graph allows — these move together. */
  | { type: 'culprit_group'; filenames: string[] }
  | { type: 'inconclusive' };

/** Serialized `bisect::BisectSession`. */
export interface BisectSession {
  schema_version: number;
  started_at: string;
  baseline_enabled: string[];
  suspects: string[];
  history: BisectStep[];
  invert_next_split: boolean;
}

/** Serialized `bisect::BisectTrial`. */
export interface BisectTrial {
  status: BisectStatus;
  enable: string[];
  disable: string[];
  completed_trials: number;
  remaining_trials: number;
}

/** A session plus the trial it currently wants, in one read. */
export interface BisectView {
  session: BisectSession | null;
  trial: BisectTrial | null;
}

export const getBisectSession = (instanceId: string) =>
  invoke<BisectView>('get_bisect_session', { instanceId });

/** Begin a bisect. `primeSuspects` are mods the crash log implicated; they are
 *  tested first, which makes the opening split much more likely to be decisive. */
export const startBisect = (instanceId: string, primeSuspects: string[] = []) =>
  invoke<BisectView>('start_bisect', { instanceId, primeSuspects });

/** Write the current trial's enable/disable set to disk, ready to launch. */
export const applyBisectTrial = (instanceId: string) =>
  invoke<BisectView>('apply_bisect_trial', { instanceId });

export const recordBisectOutcome = (instanceId: string, reproduced: boolean) =>
  invoke<BisectView>('record_bisect_outcome', { instanceId, reproduced });

/** Undo the last trial and take the other half next time. */
export const stepBackBisect = (instanceId: string) =>
  invoke<BisectView>('step_back_bisect', { instanceId });

/** End the bisect and put every mod back the way it was. */
export const cancelBisect = (instanceId: string) =>
  invoke<void>('cancel_bisect', { instanceId });

/** Group name -> assigned filenames. An entry is in at most one group. */
export type ModGroups = Record<string, string[]>;

/** Groups recorded for an instance, with names of removed content dropped. */
export const getModGroups = (instanceId: string) =>
  invoke<ModGroups>('get_mod_groups', { instanceId });

/** Assign content to a group, or pass `null` to clear the assignment. */
export const setModGroup = (instanceId: string, filenames: string[], group: string | null) =>
  invoke<ModGroups>('set_mod_group', { instanceId, filenames, group });

export const renameModGroup = (instanceId: string, from: string, to: string) =>
  invoke<ModGroups>('rename_mod_group', { instanceId, from, to });

export const deleteModGroup = (instanceId: string, group: string) =>
  invoke<ModGroups>('delete_mod_group', { instanceId, group });

/** Serialized `prune_service::PruneCategory`. */
export type PruneCategory =
  | 'libraries'
  | 'assets'
  | 'natives'
  | 'versions'
  | 'java_runtimes'
  | 'logging';

/** Serialized `prune_service::PruneCategoryReport`. The file list is
 *  deliberately not sent over IPC — only counts and totals. */
export interface PruneCategoryReport {
  category: PruneCategory;
  file_count: number;
  total_bytes: number;
}

/** Serialized `prune_service::PruneReport`. Nothing has been deleted. */
export interface PruneReport {
  categories: PruneCategoryReport[];
  /** Why a category may be reporting nothing — an unreadable instance, a
   *  malformed version JSON. Reclaim fails closed, so these explain a zero. */
  warnings: string[];
}

/** Serialized `prune_service::PruneResult`. */
export interface PruneResult {
  categories: PruneCategoryReport[];
  warnings: string[];
  total_freed_files: number;
  total_freed_bytes: number;
}

/** Dry run: what could be reclaimed from the shared runtime. Deletes nothing. */
export const scanRuntimePrune = () => invoke<PruneReport>('scan_runtime_prune', {});

/** Delete the chosen categories. */
export const runRuntimePrune = (categories: PruneCategory[]) =>
  invoke<PruneResult>('run_runtime_prune', { categories });

export const listCapturableTemplateFiles = (instanceId: string) =>
  invoke<CapturableFile[]>('list_capturable_template_files', { instanceId });

export const listInstanceTemplates = () =>
  invoke<InstanceTemplate[]>('list_instance_templates', {});

export const createInstanceTemplate = (args: {
  name: string;
  description?: string | null;
  jvm?: TemplateJvm | null;
  sourceInstanceId?: string | null;
  selectedPaths?: string[];
}) =>
  invoke<InstanceTemplate>('create_instance_template', {
    name: args.name,
    description: args.description ?? null,
    jvm: args.jvm ?? null,
    sourceInstanceId: args.sourceInstanceId ?? null,
    selectedPaths: args.selectedPaths ?? [],
  });

export const updateInstanceTemplate = (args: {
  templateId: string;
  name?: string | null;
  /** Nested option: omit to leave the description alone, pass `[value]` to set
   *  it (including `[null]` to clear). Mirrors the Rust `Option<Option<_>>`. */
  description?: [string | null] | null;
  jvm?: TemplateJvm | null;
}) =>
  invoke<InstanceTemplate>('update_instance_template', {
    templateId: args.templateId,
    name: args.name ?? null,
    description: args.description ? args.description[0] : null,
    jvm: args.jvm ?? null,
  });

export const deleteInstanceTemplate = (templateId: string) =>
  invoke<void>('delete_instance_template', { templateId });

export const applyInstanceTemplate = (instanceId: string, templateId: string) =>
  invoke<number>('apply_instance_template', { instanceId, templateId });

export const listLoadoutProfiles = (instanceId: string) =>
  invoke<LoadoutProfile[]>('list_loadout_profiles', { instanceId });

export const createLoadoutProfile = (instanceId: string, name: string) =>
  invoke<LoadoutProfile>('create_loadout_profile', { instanceId, name });

export const applyLoadoutProfile = (instanceId: string, profileName: string) =>
  invoke<void>('apply_loadout_profile', { instanceId, profileName });

export const deleteLoadoutProfile = (instanceId: string, profileName: string) =>
  invoke<void>('delete_loadout_profile', { instanceId, profileName });

export const importInstance = (sourcePath: string, symlinkSaves: boolean) =>
  invoke<ImportResult>('import_instance', { sourcePath, symlinkSaves });

export const cancelOperation = (operationId: string) =>
  invoke<boolean>('cancel_operation', { operationId });

export const detectLaunchers = () =>
  invoke<DetectedLauncher[]>('detect_launchers');

export const cloneInstance = (instanceId: string, newName: string, prefs: ClonePrefs) =>
  invoke<string>('clone_instance_cmd', { instanceId, newName, prefs });

export const exportServerEnvironment = (instanceId: string, destPath: string) =>
  invoke<ExportResult>('export_server_environment', { instanceId, destPath });

// ---------------------------------------------------------------------------
// Phase 8: Pack install
// ---------------------------------------------------------------------------

export interface PackManifest {
  name: string;
  minecraft_version: string;
  loader: string;
  loader_version: string;
  mods: PackModEntry[];
  override_source?: OverrideSource;
}

export interface PackModEntry {
  id: string;
  source: string;
  version?: string;
  status: string;
}

export interface OverrideSource {
  type: string;
  identifier: string;
  release_tag: string;
  asset_name: string;
  sha256?: string;
}

export interface PackInstallResult {
  instance_id: string;
  name: string;
  mods_installed: number;
  overrides_extracted: boolean;
}

export const installPack = (manifestJson: string, instanceId: string) =>
  invoke<PackInstallResult>('install_pack', { manifestJson, instanceId });

// --- Browse cache (Rust-backed, paginated) ---

export interface ScoreBreakdown {
  score: number;
  popularity: number;
  downloadsNorm: number;
  endorsementsNorm: number;
  libraryPenalized: boolean;
  voteNudge: number;
  curated: boolean;
}

export interface BrowseItemCached {
  id: string;
  source: string;
  registryItem: RegistryItem | null;
  modrinthResult: ModrinthSearchResult | null;
  technicResult?: TechnicSearchResult | null;
  /** Unified 0-100 cross-source score used to rank the merged list. */
  score?: number;
  scoreBreakdown?: ScoreBreakdown | null;
  name: string;
  iconUrl: string | null;
  description: string | null;
  contentType: string;
  heroImageUrl: string | null;
  author: string | null;
  categories: string[];
  downloads: number | null;
  follows: number | null;
  upvotes: number | null;
  downvotes: number | null;
  netScore: number | null;
  supportedVersions: string[];
  sourcePageUrl: string | null;
}

export interface BrowsePage {
  items: BrowseItemCached[];
  total: number;
  page: number;
  hasMore: boolean;
}

export const browseSearch = (
  queryKey: string,
  query?: string,
  contentType?: string,
  category?: string,
  sort?: string,
  mcVersion?: string,
  loader?: string,
) =>
  invoke<BrowsePage>('browse_search', {
    queryKey,
    query: query ?? null,
    contentType: contentType ?? null,
    category: category ?? null,
    sort: sort ?? null,
    mcVersion: mcVersion ?? null,
    loader: loader ?? null,
  });

export const browseLoadMore = (queryKey: string, pageIndex: number) =>
  invoke<BrowsePage>('browse_load_more', { queryKey, pageIndex });

export const browsePage = (queryKey: string, page: number) =>
  invoke<BrowsePage>('browse_page', { queryKey, page });

// --- Repair loader ---

export interface InstallReceiptSummary {
  tuple: { loader: string; minecraft_version: string; loader_version: string };
  profile_id: string;
  cache_hit: boolean;
  profile_stable_hash: string;
  receipt_schema_version: number;
  installer_exit_status: number;
}

/** Force-reinstall the loader for an instance. Returns the install receipt. */
export const repairInstanceLoader = (instanceId: string) =>
  invoke<InstallReceiptSummary>('repair_instance_loader', { instanceId });

// ---------------------------------------------------------------------------
// Stage 3: Managed Java runtime
// ---------------------------------------------------------------------------

/**
 * Summary of a detected or managed Java runtime.
 * Mirrors Rust `JavaRuntimeSummary` / `JavaInstallation`.
 */
export interface JavaRuntimeSummary {
  path: string;
  version: number;
  version_string: string;
  source: string;
  arch: string | null;
}

/** List all discovered Java runtimes (managed + Mojang + system). */
export const listJavaRuntimes = () =>
  invoke<JavaRuntimeSummary[]>('list_java_runtimes');

/**
 * Ensure a managed Java runtime for the given major version is installed.
 * Provisions from the embedded Adoptium catalog when missing.
 * Returns the provisioned runtime summary.
 *
 * @param operationId - Optional operation ID for cancellation tracking.
 *   When omitted, a stable key `"settings-{major}"` is used by the backend.
 */
export const ensureJavaRuntime = (major: number, operationId?: string) =>
  invoke<JavaRuntimeSummary>('ensure_java_runtime', { major, operationId: operationId ?? null });

/**
 * Remove unused managed Java runtimes (keep newest per major).
 * Returns the number of runtimes that were removed.
 */
export const removeUnusedJavaRuntimes = () =>
  invoke<number>('remove_unused_java_runtimes');

/**
 * Inspect a Java executable at the given path and return its summary.
 * Used for picker validation before the user saves a custom Java path.
 */
export const inspectJavaExecutable = (path: string) =>
  invoke<JavaRuntimeSummary>('inspect_java_executable', { path });

/**
 * Update per-instance Java path, custom JVM arguments, and incompatible override setting.
 * Pass path as null/undefined to clear the per-instance override.
 * Omit customArgs when changing only the Java runtime selection.
 */
export const updateInstanceJava = (
  instanceId: string,
  path: string | null,
  allowIncompatible: boolean,
  customArgs?: string,
) =>
  invoke<void>('update_instance_java', {
    instanceId,
    path,
    allowIncompatible,
    customArgs: customArgs ?? null,
  });

export const updateInstanceJvm = (
  instanceId: string,
  memoryMb: number,
  gc: string,
  alwaysPreTouch: boolean,
  customArgs: string,
  memoryMode: 'auto' | 'manual',
) =>
  invoke<void>('update_instance_jvm', {
    instanceId,
    memoryMb,
    gc,
    alwaysPreTouch,
    customArgs,
    memoryMode,
  });

export interface MemoryRecommendation {
  recommended_mb: number;
  tier_label: string;
  tier_index: number;
  is_large_resource_pack_adjustment: boolean;
  ram_capped: boolean;
  insufficient_system_ram: boolean;
  system_ram_mb: number;
  next_tier_mb: number;
  next_tier_label: string;
  factors: string[];
  explanation: string;
}

export const recommendInstanceMemory = (instanceId: string) =>
  invoke<MemoryRecommendation>('recommend_instance_memory', { instanceId });

/**
 * Structured error details for JavaRuntimeMissing.
 * Available when the LauncherError code is 'ERR_JAVA_RUNTIME_MISSING'.
 */
export interface JavaRuntimeMissingDetails {
  major: number;
  component: string;
  suggested_actions: Array<'download_runtime' | 'choose_java' | 'cancel'>;
}

/**
 * Structured error details for JavaRuntimeCatalogMissing.
 * Available when the LauncherError code is 'ERR_JAVA_RUNTIME_CATALOG_MISSING'.
 */
export interface JavaRuntimeCatalogMissingDetails {
  major: number;
  os: string;
  arch: string;
  suggested_actions: Array<'choose_java' | 'cancel'>;
}

/**
 * Progress event payload for java-runtime-progress events.
 * Emitted by the backend during runtime provisioning.
 */
export interface JavaRuntimeProgressEvent {
  instance_id: string;
  major: number;
  stage: 'ensuring' | 'downloading' | 'ready' | string;
  message: string;
  percent: number;
}

/**
 * Cancel a Java runtime provisioning operation by its operation ID.
 */
export const cancelJavaRuntime = (operationId: string) =>
  invoke<void>('cancel_java_runtime', { operationId });

/**
 * Structured error details for JavaRuntimeDownloadDisabled.
 * Available when the LauncherError code is 'ERR_JAVA_RUNTIME_DOWNLOAD_DISABLED'.
 */
export interface JavaRuntimeDownloadDisabledDetails {
  major: number;
  component: string;
  suggested_actions: Array<'choose_java' | 'open_privacy' | 'cancel'>;
}

/** Serialized `launch_history::LaunchResult`. */
export type LaunchHistoryOutcome = 'ok' | 'crashed' | 'unknown';

/** Serialized `launch_history::LaunchRecord`. */
export interface LaunchRecord {
  id: number;
  instance_id: string;
  started_at: string;
  /** Agora's own preparation time before the process started. */
  prep_ms: number | null;
  /** Session length. `null` while still running. */
  duration_ms: number | null;
  outcome: LaunchHistoryOutcome | null;
  enabled_mod_count: number;
  minecraft_version: string;
  loader: string;
  peak_memory_mb: number | null;
}

/** Serialized `launch_history::LaunchStats`. The recent/earlier pair is what
 *  lets the UI say "startup got slower" without over-reading one cold start. */
export interface LaunchStats {
  runs: number;
  crashes: number;
  median_prep_ms: number | null;
  recent_median_prep_ms: number | null;
  earlier_median_prep_ms: number | null;
  latest_mod_count: number | null;
  earliest_mod_count: number | null;
}

export interface LaunchHistoryView {
  records: LaunchRecord[];
  stats: LaunchStats;
}

/** Recorded launches for an instance. Local only — no endpoint, deleted with
 *  the instance. */
export const getLaunchHistory = (instanceId: string) =>
  invoke<LaunchHistoryView>('get_launch_history', { instanceId });

/** Serialized `commands::SharedScreenshotStatus`. */
export interface SharedScreenshotStatus {
  linked: boolean;
  target: string | null;
  shared_root: string;
}

export const getSharedScreenshotStatus = (instanceId: string) =>
  invoke<SharedScreenshotStatus>('get_shared_screenshot_status', { instanceId });

/** Point this instance's screenshots at the shared folder. Existing files are
 *  moved across, never discarded; a name collision refuses. */
export const linkSharedScreenshots = (instanceId: string) =>
  invoke<string>('link_shared_screenshots', { instanceId });

/** Stop sharing. The shared screenshots themselves are left alone. */
export const unlinkSharedScreenshots = (instanceId: string) =>
  invoke<void>('unlink_shared_screenshots', { instanceId });

/** Create a desktop shortcut that launches this instance directly. Returns the
 *  shortcut's path. Clicking it starts Agora on that instance, or tells an
 *  already-running Agora to launch it. */
export const createDesktopShortcut = (instanceId: string, displayName: string) =>
  invoke<string>('create_desktop_shortcut', { instanceId, displayName });
