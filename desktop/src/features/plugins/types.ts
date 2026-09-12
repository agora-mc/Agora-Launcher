/**
 * The plugin contract, as TypeScript.
 *
 * These mirror `crates/agora-plugin-api` exactly. They are hand-written rather
 * than generated because the set is small and stable, and because the comments
 * that matter here — which field is a *namespaced* id, which string is really
 * an enum — do not survive generation.
 *
 * Nothing in a `ViewModel` is markup. That is the point: community content
 * reaches the screen as data the host renders with its own components, so
 * there is never a string for `dangerouslySetInnerHTML` to receive.
 */

export type Tone = 'neutral' | 'info' | 'success' | 'warning' | 'danger';

export type ContributionKind =
  | 'page'
  | 'instance-panel'
  | 'command'
  | 'setting'
  | 'diagnostic'
  | 'launch-check'
  | 'theme'
  | 'replacement';

/** A contribution with its plugin namespace applied: `publisher.plugin/local`. */
export interface NamespacedContribution {
  id: string;
  pluginId: string;
  localId: string;
  kind: ContributionKind;
  title: string;
}

export type PluginStatus =
  | { state: 'ready' }
  | { state: 'disabled' }
  | { state: 'incompatibleApi'; detail: { required: string; host: string } }
  | { state: 'unresolvedDependency'; detail: { dependency: string; reason: string } }
  | { state: 'dependencyCycle'; detail: { cycle: string[] } }
  | { state: 'failed'; detail: { message: string } };

export interface ReplacementOffer {
  id: string;
  pluginId: string;
  localId: string;
  pluginName: string;
  title: string;
  description: string | null;
  surface: string;
  /** The plugin export that builds the view. */
  export: string;
}

export interface SurfaceChoice {
  surface: string;
  title: string;
  /** Every offer from an installed plugin, runnable or not. */
  offers: ReplacementOffer[];
  /** What the user picked, even if it cannot currently render. */
  selected: string | null;
  /** What will actually render. `null` means Agora's own view. */
  effective: ReplacementOffer | null;
  /**
   * Set when a selection exists but is not what renders, with the reason.
   * This is the difference between "you chose the built-in" and "your choice
   * is broken and we quietly did something else".
   */
  fallbackReason: string | null;
}

export interface KeyFingerprint {
  id: string;
  fingerprint: string;
}

export interface UpdateSourceSummary {
  url: string;
  host: string;
  keys: KeyFingerprint[];
}

export interface UpdateCheckRecord {
  at: string;
  result: string;
}

export interface PluginSummary {
  id: string;
  name: string;
  version: string;
  description: string | null;
  license: string;
  sourceUrl: string | null;
  enabled: boolean;
  running: boolean;
  development: boolean;
  status: PluginStatus;
  /** Already phrased for a human; prefer this over re-deriving from `status`. */
  statusText: string;
  capabilities: string[];
  declaredHosts: string[];
  contributions: NamespacedContribution[];
  definitions: PluginDefinitions;
  installedAt: string;
  updatedAt: string;
  updateSource: UpdateSourceSummary | null;
  lastUpdateCheck: UpdateCheckRecord | null;
  droppedEvents: number;
}

export interface CapabilityDescription {
  name: string;
  summary: string;
  isMutating: boolean;
}

export interface InstallPreview {
  manifest: PluginManifest;
  requiredCapabilities: CapabilityDescription[];
  optionalCapabilities: CapabilityDescription[];
  unsupportedCapabilities: string[];
  replacesVersion: string | null;
  /**
   * Permissions this package wants that are not already granted. On a first
   * install that is everything; on a replacement it is only what is new, which
   * is the part the user is actually being asked about.
   */
  addedCapabilities: string[];
  /** Hosts this package would reach that the installed version could not. */
  addedHosts: string[];
  updateSource: UpdateSourceSummary | null;
  migratesData: boolean;
  fileCount: number;
  uncompressedBytes: number;
}

export type UpdateVerdict =
  | { state: 'upToDate' }
  | {
      state: 'available';
      from: string;
      to: string;
      notes: string | null;
      url: string;
      sha256: string;
      size: number;
    }
  | { state: 'needsNewerHost'; latest: string; requires: string }
  | { state: 'installedIsNewer'; installed: string; latest: string }
  | { state: 'noReleases' };

export type UpdateOutcome =
  | { outcome: 'installed'; plugin: PluginSummary }
  | { outcome: 'needsConsent'; preview: InstallPreview };

export interface SettingDefinition {
  key: string;
  title: string;
  description?: string | null;
  type: 'boolean' | 'string' | 'number' | 'enum';
  default?: unknown;
  min?: number;
  max?: number;
  maxLength?: number;
  options?: { value: string; label: string }[];
}

export interface PluginManifest {
  manifest: number;
  id: string;
  name: string;
  version: string;
  description?: string | null;
  license: string;
  source?: string | null;
  apiRange: string;
  entrypoint?: string | null;
  network?: { hosts: string[] };
  contributions?: {
    pages?: { id: string; title: string; icon?: string | null }[];
    instancePanels?: { id: string; title: string; icon?: string | null }[];
    commands?: { id: string; title: string; icon?: string | null }[];
    settings?: SettingDefinition[];
    theme?: { id: string; title: string; light?: Record<string, string>; dark?: Record<string, string> };
    diagnostics?: { id: string; title: string; description?: string | null }[];
    launchChecks?: { id: string; title: string }[];
  };
}

/** Host-rendered only. The custom-HTML prototype was withdrawn in API 0.1. */
export type ViewSource = { kind: 'host'; export: string };
export interface PluginDefinitions {
  pages: { id: string; title: string; view: ViewSource }[];
  instancePanels: { id: string; title: string; view: ViewSource }[];
  commands: { id: string; title: string; export: string; surfaces: ('palette' | 'instance-context')[] }[];
  diagnostics: { id: string; title: string; export: string }[];
  launchChecks: { id: string; title: string; export: string }[];
  theme?: { id: string; title: string; light: Record<string, string>; dark: Record<string, string> };
}

// ---------------------------------------------------------------------------
// Host-rendered views
// ---------------------------------------------------------------------------

export interface Stat {
  label: string;
  value: string;
  hint?: string | null;
  tone?: Tone;
}

export interface Column {
  label: string;
  align?: 'start' | 'end';
}

export type Cell =
  | { type: 'text'; text: string }
  | { type: 'badge'; text: string; tone?: Tone }
  | { type: 'flag'; value: boolean };

export interface ListItem {
  title: string;
  detail?: string | null;
  tone?: Tone;
}

export interface ActionButton {
  id: string;
  label: string;
  /** The plugin's own export. Never a host method: a plugin cannot draw a
   *  button that performs an operation the plugin itself may not request. */
  export: string;
  args?: unknown;
  tone?: Tone;
  confirm?: string | null;
}

export type ViewBlock =
  | { type: 'heading'; text: string }
  | { type: 'text'; text: string; tone?: Tone }
  | { type: 'stats'; items: Stat[] }
  | { type: 'table'; columns: Column[]; rows: Cell[][]; emptyMessage?: string | null }
  | { type: 'list'; items: ListItem[] }
  | { type: 'status'; tone: Tone; title: string; message?: string | null }
  | { type: 'actions'; items: ActionButton[] }
  | { type: 'divider' };

export interface ViewModel {
  title?: string | null;
  subtitle?: string | null;
  blocks: ViewBlock[];
}

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

export type Severity = 'info' | 'warning' | 'error';

export interface Evidence {
  label: string;
  value: string;
}

export type RepairAction =
  | { action: 'disableContent'; instanceId: string; key: string }
  | { action: 'enableContent'; instanceId: string; key: string }
  | { action: 'pinContentUpdate'; instanceId: string; key: string }
  | { action: 'unpinContentUpdate'; instanceId: string; key: string }
  | { action: 'setJvmMemory'; instanceId: string; memoryMb: number }
  | { action: 'resetJvmArgs'; instanceId: string }
  | { action: 'createSnapshot'; instanceId: string; label?: string | null };

export interface RepairProposal {
  id: string;
  title: string;
  description?: string | null;
  actions: RepairAction[];
}

export interface Finding {
  id: string;
  title: string;
  severity: Severity;
  summary?: string | null;
  evidence: Evidence[];
  repairs: RepairProposal[];
}

export interface DiagnosticReport {
  findings: Finding[];
  /** Set when the plugin could not finish. A partial report with an honest
   *  note is more useful than a clean one that checked nothing. */
  incompleteReason?: string | null;
}

export interface RepairFailure {
  action: string;
  message: string;
  retryable: boolean;
}

export interface RepairOutcome {
  applied: string[];
  failed: RepairFailure[];
  /** Actions skipped because the world changed since the plugin proposed them. */
  stale: string[];
}

export interface LaunchCheckResult {
  pluginId: string;
  checkId: string;
  title: string;
  report: DiagnosticReport;
  blocking: boolean;
}

export interface LaunchCheckOutcome {
  results: LaunchCheckResult[];
  /** Checks that failed to run. Reported, never treated as a pass. */
  errors: string[];
}

export interface PluginStartFailure {
  pluginId: string;
  message: string;
}

/** Payload of the `plugin-notify` event. */
export interface PluginNotification {
  pluginId: string;
  tone: Tone;
  message: string;
}

/** Payload of the `plugin-refresh` event. */
export interface PluginRefresh {
  pluginId: string;
  viewId: string | null;
}

/** Split `publisher.plugin/local` back into its two halves. */
export function splitContributionId(id: string): { pluginId: string; localId: string } | null {
  const slash = id.indexOf('/');
  if (slash <= 0 || slash === id.length - 1) return null;
  return { pluginId: id.slice(0, slash), localId: id.slice(slash + 1) };
}
