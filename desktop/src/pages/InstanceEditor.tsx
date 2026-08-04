import { useEffect, useState, useRef } from 'react';
import { useAdvancedMode } from '../components/AdvancedModeContext';
import { ConsoleView } from '../components/ConsoleView';
import { InstallFlow } from '../components/InstallFlow';
import { LauncherImportWizard } from '../components/LauncherImportWizard';
import { DependencyPrompt } from '../components/DependencyPrompt';
import { PackInstallProgressBar, usePackInstall } from '../components/PackInstallProgress';
import type { BatchInstallItem, InstallIntent } from '../lib/installFlow';
import {
  getInstanceDetail,
  listInstanceContent,
  enrichInstanceContent,
  enableInstanceMod,
  disableInstanceMod,
  getDisablePlan,
  checkInstanceUpdates,
  exportInstancePack,
  formatError,
  inspectJavaExecutable,
  pickOpenFile,
  getCustomIcon,
  setCustomInstanceIcon,
  setCustomModIcon,
  importInstance,
  exportLockfile,
  verifyLockfile,
  repairLockfile,
  importLockfile,
  updateInstanceJava,
  updateInstanceJvm,
  computeGcArgs,
  recommendInstanceMemory,
  browseItems,
  listModVersions,
  listPackMods,
  unlockInstance,
  lockInstance,
  renameInstance,
  revertInstance,
  listSnapshots,
  createSnapshot,
  restoreSnapshot,
  deleteSnapshot,
  detectDrift,
  listLoadoutProfiles,
  createLoadoutProfile,
  applyLoadoutProfile,
  deleteLoadoutProfile,
  openInstanceFolder,
  revealPath,
  type InstanceDetail,
  type InstanceManifest,
  type JavaRuntimeSummary,
  type GcProfile,
  type RegistryItem,
  type PackModRow,
  type InstalledMod,
  type InstalledContentRow,
  type DisablePlan,
  type DependentInfo,
  type UpdateInfo,
  type Snapshot,
  type SnapshotDiff,
  type LoadoutProfile,
  type LockfileDriftReport,
  type MemoryRecommendation,
  type HealthReport,
} from '../lib/tauri';
import { InstalledContentPanel } from '../components/installed-content/InstalledContentPanel';
import { formatInstalledDate } from '../components/installed-content/contentTableState';
import { ImagePlus, Play } from 'lucide-react';

function installedModKey(mod: InstalledMod): string {
  return `${mod.filename}:${mod.sha256}`;
}

function installedModDetailId(mod: InstalledMod): string | null {
  return mod.registry_id || mod.modrinth_id || mod.mod_jar_id || null;
}

function installedModSourceLabel(source: string): string {
  const normalized = (source ?? '').trim().toLowerCase().replace(/[\s-]+/g, '_');
  if (normalized === 'modrinth_raw' || normalized === 'modrinth') return 'Modrinth';
  if (normalized === 'modrinth_pack') return 'Modrinth Pack';
  if (normalized.includes('github')) return 'GitHub Release';
  if (normalized === 'registry' || normalized === 'curated') return 'Agora Registry';
  if (normalized.includes('manual') || normalized === 'local') return 'Manual';
  return 'Other';
}

const CONTENT_KEYS = ['mods', 'resourcepacks', 'shaders', 'datapacks', 'worlds'] as const;

function updateManifestEntryEnabled(
  manifest: InstanceManifest,
  filename: string,
  enabled: boolean,
): InstanceManifest {
  for (const key of CONTENT_KEYS) {
    if (!manifest[key].some((entry) => entry.filename === filename)) continue;
    return {
      ...manifest,
      [key]: manifest[key].map((entry) =>
        entry.filename === filename ? { ...entry, enabled } : entry,
      ),
    };
  }
  return manifest;
}

function installedModMetadataKey(mods: InstalledMod[] | undefined): string {
  return (mods ?? [])
    .map((mod) => `${installedModKey(mod)}:${installedModDetailId(mod) ?? ''}`)
    .join('|');
}

function fallbackContentRows(manifest: InstanceManifest | null): InstalledContentRow[] {
  if (!manifest) return [];
  return manifest.mods
    .concat(manifest.resourcepacks, manifest.shaders, manifest.datapacks, manifest.worlds)
    .map((entry) => ({
      key: `${entry.content_type}:${entry.filename}:${entry.sha256}`,
      filename: entry.filename,
      display_name: entry.filename.replace(/\.[^.]+$/, ''),
      version: entry.version,
      content_type: entry.content_type,
      enabled: entry.enabled,
      installed_at: entry.installed_at,
      source: entry.source,
      source_label: installedModSourceLabel(entry.source),
      source_url: entry.source_url ?? null,
      registry_id: entry.registry_id,
      modrinth_id: entry.modrinth_id,
      mod_jar_id: entry.mod_jar_id ?? null,
      loader_mod_id: entry.mod_jar_id ?? null,
      size_bytes: null,
      file_present: false,
      resolved_path: null,
      author: null,
      categories: ['Uncategorized'],
      icon_url: null,
      curation_status: 'unknown' as const,
      agora_score: null,
      modrinth_downloads: null,
      metadata_status: 'unavailable' as const,
    }));
}

function safeIconUrl(value: unknown): string | null {
  return typeof value === 'string' && value.startsWith('https://') ? value : null;
}

function packIconUrl(manifest: InstanceManifest | null | undefined): string | null {
  return safeIconUrl(manifest?.user_preferences?.agora_pack_icon_url);
}

function customModIconMap(manifest: InstanceManifest | null): Record<string, unknown> {
  const value = manifest?.user_preferences?.agora_custom_mod_icons;
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

type GcMode = 'auto' | GcProfile;

function storedGcMode(value: string | undefined): GcMode {
  switch ((value ?? '').toLowerCase()) {
    case 'zgc':
    case 'low_latency':
      return 'low_latency';
    case 'manual':
      return 'manual';
    case 'g1gc':
      // Legacy rows used g1gc as the implicit default before Auto existed.
      return 'auto';
    case 'high_efficiency':
      return 'high_efficiency';
    default:
      return 'auto';
  }
}

function previewJavaMajor(version: string | undefined): number {
  const parts = (version ?? '').split('.');
  const first = Number(parts[0]);
  if (first >= 26) return 25;
  const minor = first === 1 ? Number(parts[1]) : first;
  const patch = first === 1 ? Number(parts[2]) : Number(parts[1]);
  if (minor >= 21 || (minor === 20 && patch >= 5)) return 21;
  if (minor >= 18) return 17;
  return 8;
}

export function InstanceEditor({ instanceId, onBack, onOpenInstanceEditor, onOpenModDetail, onOpenBrowseForInstance, onLaunch, onInvestigate, processLogs, processState, onKillProcess, healthReport, onReviewHealth }: { instanceId: string; onBack: () => void; onOpenInstanceEditor?: (instanceId: string) => void; onOpenModDetail?: (itemId: string) => void; onOpenBrowseForInstance?: (instanceId: string, contentType?: string) => void; onLaunch?: (instanceId: string) => Promise<boolean>; onInvestigate?: (instanceId: string) => void; processLogs?: import('../lib/useProcessController').LogLine[]; processState?: import('../lib/useProcessController').ProcessState; onKillProcess?: () => Promise<void>; healthReport?: HealthReport | null; onReviewHealth?: (instanceId: string, instanceName: string, report: HealthReport) => void }) {
  const [detail, setDetail] = useState<InstanceDetail | null>(null);
  const [contentRows, setContentRows] = useState<InstalledContentRow[]>([]);
  const [contentRowsLoaded, setContentRowsLoaded] = useState(false);
  const [contentAuthors, setContentAuthors] = useState<Record<string, string>>({});
  const [contentDisplayNames, setContentDisplayNames] = useState<Record<string, string>>({});
  const [contentIcons, setContentIcons] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [exportBusy, setExportBusy] = useState(false);
  const [instanceCustomIcon, setInstanceCustomIcon] = useState<string | null>(null);
  const [modCustomIcons, setModCustomIcons] = useState<Record<string, string>>({});

  const { advancedMode } = useAdvancedMode();
  const { getTaskForInstance, revision: packInstallRevision, startPackFile, startPlan } = usePackInstall();

  // Sub-sidebar active tab
  const [activeTab, setActiveTab] = useState<'mods' | 'resourcepacks' | 'shaders' | 'datapacks' | 'snapshots' | 'loadout-profiles' | 'import' | 'export' | 'console' | 'java-args'>('mods');

  // Snapshots state (Phase 6)
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  const [snapshotLabelInput, setSnapshotLabelInput] = useState('');
  const [snapshotBusy, setSnapshotBusy] = useState<string | null>(null);
  const [confirmDeleteSnapshot, setConfirmDeleteSnapshot] = useState<string | null>(null);
  const [snapshotDiff, setSnapshotDiff] = useState<{ snapshotId: string; diff: SnapshotDiff } | null>(null);

  // Loadout profiles state (Phase 6)
  const [profiles, setProfiles] = useState<LoadoutProfile[]>([]);
  const [profileNameInput, setProfileNameInput] = useState('');
  const [profileBusy, setProfileBusy] = useState<string | null>(null);
  const [confirmDeleteProfile, setConfirmDeleteProfile] = useState<string | null>(null);

  // Import state (Phase 6)
  const [importBusy, setImportBusy] = useState(false);
  const [launcherImportOpen, setLauncherImportOpen] = useState(false);
  const [lockfileText, setLockfileText] = useState('');
  const [lockfileBusy, setLockfileBusy] = useState<'export' | 'verify' | 'repair' | 'clone' | 'copy' | null>(null);
  const [lockfileReport, setLockfileReport] = useState<LockfileDriftReport | null>(null);
  const [lockfileNotice, setLockfileNotice] = useState<string | null>(null);

  // Java & Args state
  const [instanceJavaPath, setInstanceJavaPath] = useState('');
  const [instanceJavaArgs, setInstanceJavaArgs] = useState('');
  const [instanceJvmMemory, setInstanceJvmMemory] = useState(4096);
  const [instanceMemoryMode, setInstanceMemoryMode] = useState<'auto' | 'manual'>('manual');
  const [memoryRecommendation, setMemoryRecommendation] = useState<MemoryRecommendation | null>(null);
  const [instanceGcMode, setInstanceGcMode] = useState<GcMode>('auto');
  const [instanceAlwaysPreTouch, setInstanceAlwaysPreTouch] = useState(true);
  const [gcPreview, setGcPreview] = useState<Awaited<ReturnType<typeof computeGcArgs>> | null>(null);
  const [gcPreviewLoading, setGcPreviewLoading] = useState(false);
  const [instanceJavaInspected, setInstanceJavaInspected] = useState<JavaRuntimeSummary | null>(null);
  const [instanceJavaInspectError, setInstanceJavaInspectError] = useState<string | null>(null);
  const [instanceJavaAllowOverride, setInstanceJavaAllowOverride] = useState(false);
  const [instanceJavaSaving, setInstanceJavaSaving] = useState(false);
  const [playBusy, setPlayBusy] = useState(false);

  const [canonicalOperation, setCanonicalOperation] = useState<{
    intent: InstallIntent;
    instanceName: string;
  } | null>(null);
  const [disablePlanTarget, setDisablePlanTarget] = useState<{
    rows: InstalledContentRow[];
    candidates: { key: string; dependent: DependentInfo }[];
  } | null>(null);

  // Pack install state
  const [packInstallOpen, setPackInstallOpen] = useState(false);
  const [packIdInput, setPackIdInput] = useState('');
  const [packProgress, setPackProgress] = useState<
    { modId: string; status: 'pending' | 'installing' | 'done' | 'failed'; error?: string }[] | null
  >(null);
  const [availablePacks, setAvailablePacks] = useState<RegistryItem[]>([]);
  const [packDropdownOpen, setPackDropdownOpen] = useState(false);
  const packDropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!packInstallOpen) return;
    let cancelled = false;
    browseItems('pack').then((packs) => {
      if (!cancelled) setAvailablePacks(packs);
    }).catch(() => {});
    return () => { cancelled = true; };
  }, [packInstallOpen]);

  // Close pack dropdown on outside click
  useEffect(() => {
    if (!packDropdownOpen) return;
    const handler = (e: MouseEvent) => {
      if (packDropdownRef.current && !packDropdownRef.current.contains(e.target as Node)) {
        setPackDropdownOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [packDropdownOpen]);

  useEffect(() => {
    setContentRowsLoaded(false);
    setContentAuthors({});
    setContentDisplayNames({});
    setContentIcons({});
    let cancelled = false;
    (async () => {
      try {
        const result = await getInstanceDetail(instanceId);
        if (!cancelled) {
          setDetail(result);
          if (result?.row.icon_path) {
            void getCustomIcon(instanceId, 'instance').then((icon) => {
              if (!cancelled) setInstanceCustomIcon(icon);
            }).catch(() => {
              if (!cancelled) setInstanceCustomIcon(null);
            });
          } else {
            setInstanceCustomIcon(null);
          }
          setInstanceJavaPath(result?.row?.java_path ?? '');
          setInstanceJavaArgs(result?.row?.jvm_custom_args ?? '');
          setInstanceJvmMemory(result?.row?.jvm_memory_mb ?? 4096);
          setInstanceMemoryMode(result?.row?.jvm_memory_mode ?? 'manual');
          setInstanceGcMode(storedGcMode(result?.row?.jvm_gc));
          setInstanceAlwaysPreTouch(result?.row?.jvm_always_pre_touch ?? true);
          setInstanceJavaAllowOverride(result?.row?.java_incompatible_override ?? false);
          if (!result) setError('Instance not found.');
        }
      } catch (e) {
        if (!cancelled) setError(formatError(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [instanceId]);

  const refreshContent = async () => {
    const result = await listInstanceContent(instanceId);
    if (!Array.isArray(result)) throw new Error('Installed content inventory was unavailable.');
    setContentRows((current) => {
      const existingAuthors = new Map(current.map((row) => [row.key, row.author]));
      return result.map((row) => ({
        ...row,
        display_name: contentDisplayNames[row.key] ?? row.display_name,
        icon_url: row.icon_url ?? contentIcons[row.key] ?? null,
        author: row.author ?? contentAuthors[row.key] ?? existingAuthors.get(row.key) ?? null,
      }));
    });
    setContentRowsLoaded(true);
    try {
      const metadata = await enrichInstanceContent(instanceId);
      if (Array.isArray(metadata)) applyContentMetadata(metadata);
    } catch {
      // Local inventory remains usable when Modrinth is offline or disabled.
    }
  };

  const applyContentMetadata = (metadata: { key: string; display_name: string | null; icon_url: string | null; author: string | null }[]) => {
    const displayNames = Object.fromEntries(
      metadata
        .filter((item): item is { key: string; display_name: string; icon_url: string | null; author: string | null } => Boolean(item.display_name))
        .map((item) => [item.key, item.display_name] as const),
    );
    const authors = Object.fromEntries(
      metadata
        .filter((item) => Boolean(item.author))
        .map((item) => [item.key, item.author as string] as const),
    );
    const icons = Object.fromEntries(
      metadata
        .filter((item) => Boolean(item.icon_url))
        .map((item) => [item.key, item.icon_url as string] as const),
    );
    setContentDisplayNames(displayNames);
    setContentIcons(icons);
    setContentAuthors(authors);
    setContentRows((current) => current.map((row) => ({
      ...row,
      display_name: displayNames[row.key] ?? row.display_name,
      icon_url: row.icon_url ?? icons[row.key] ?? null,
      author: row.author ?? authors[row.key] ?? null,
    })));
  };

  useEffect(() => {
    let cancelled = false;
    void listInstanceContent(instanceId)
      .then((result) => {
        if (!cancelled && Array.isArray(result)) {
          setContentRows(result);
          setContentRowsLoaded(true);
          void enrichInstanceContent(instanceId)
            .then((metadata) => {
              if (!cancelled && Array.isArray(metadata)) applyContentMetadata(metadata);
            })
            .catch(() => {});
        }
      })
      .catch(() => {
        // Existing manifest data remains usable when the inventory command is
        // unavailable during an older-webview upgrade or test fixture.
      });
    return () => { cancelled = true; };
  }, [instanceId]);

  useEffect(() => {
    if (packInstallRevision === 0) return;
    void getInstanceDetail(instanceId)
      .then((result) => {
        setDetail(result);
        return refreshContent();
      })
      .catch((cause) => setError(formatError(cause)));
  }, [instanceId, packInstallRevision]);

  useEffect(() => {
    if (detail?.snapshot_readiness !== 'pending') return undefined;
    const timer = window.setInterval(() => {
      void getInstanceDetail(instanceId)
        .then((result) => setDetail(result))
        .catch(() => {});
    }, 1500);
    return () => window.clearInterval(timer);
  }, [detail?.snapshot_readiness, instanceId]);

  useEffect(() => {
    if (activeTab !== 'java-args') return;
    let cancelled = false;
    void recommendInstanceMemory(instanceId)
      .then((recommendation) => {
        if (!cancelled) setMemoryRecommendation(recommendation);
      })
      .catch(() => {
        if (!cancelled) setMemoryRecommendation(null);
      });
    return () => { cancelled = true; };
  }, [activeTab, instanceId, detail?.manifest?.mods, detail?.manifest?.resourcepacks]);

  useEffect(() => {
    if (activeTab !== 'java-args' || !detail?.row) return;
    let cancelled = false;
    setGcPreviewLoading(true);
    computeGcArgs(
      instanceJavaInspected?.version ?? previewJavaMajor(detail.row.minecraft_version),
      instanceMemoryMode === 'auto' && memoryRecommendation
        ? memoryRecommendation.recommended_mb
        : instanceJvmMemory,
      instanceGcMode === 'manual' ? instanceJavaArgs : '',
      instanceGcMode,
      instanceAlwaysPreTouch,
    ).then((result) => {
      if (!cancelled) setGcPreview(result);
    }).catch(() => {
      if (!cancelled) setGcPreview(null);
    }).finally(() => {
      if (!cancelled) setGcPreviewLoading(false);
    });
    return () => { cancelled = true; };
  }, [activeTab, detail?.row, instanceGcMode, instanceJvmMemory, instanceMemoryMode, memoryRecommendation, instanceJavaArgs, instanceAlwaysPreTouch, instanceJavaInspected?.version]);

  const modMetadataKey = installedModMetadataKey(detail?.manifest?.mods);

  const customModIconsKey = JSON.stringify(customModIconMap(detail?.manifest ?? null));
  useEffect(() => {
    const installedMods = detail?.manifest?.mods;
    const storedIcons = customModIconMap(detail?.manifest ?? null);
    if (!installedMods) return;
    let cancelled = false;
    void Promise.all(installedMods.map(async (mod) => {
      if (typeof storedIcons[mod.filename] !== 'string') return null;
      try {
        const icon = await getCustomIcon(instanceId, 'mod', mod.filename);
        return icon ? [installedModKey(mod), icon] as const : null;
      } catch {
        return null;
      }
    })).then((results) => {
      if (cancelled) return;
      setModCustomIcons(Object.fromEntries(results.filter((result): result is readonly [string, string] => result !== null)));
    });
    return () => { cancelled = true; };
  }, [instanceId, customModIconsKey, modMetadataKey]);

  // Load snapshots when tab becomes active
  useEffect(() => {
    if (activeTab !== 'snapshots') return;
    let cancelled = false;
    (async () => {
      try {
        const result = await listSnapshots(instanceId);
        if (!cancelled) setSnapshots(result);
      } catch (e) {
        if (!cancelled) setError(formatError(e));
      }
    })();
    return () => { cancelled = true; };
  }, [instanceId, activeTab]);

  // Load loadout profiles when tab becomes active
  useEffect(() => {
    if (activeTab !== 'loadout-profiles') return;
    let cancelled = false;
    (async () => {
      try {
        const result = await listLoadoutProfiles(instanceId);
        if (!cancelled) setProfiles(result);
      } catch (e) {
        if (!cancelled) setError(formatError(e));
      }
    })();
    return () => { cancelled = true; };
  }, [instanceId, activeTab]);

  const beginCanonicalOperation = (action: InstallIntent['action']) => {
    setCanonicalOperation({
      instanceName: detail?.row.name ?? instanceId,
      intent: {
        action,
        targetInstance: instanceId,
        optionalDeps: { type: 'prompt' },
        requestedBy: 'interactive',
        overrides: {
          allowReplace: false,
          skipHealthScan: false,
          forceConflictResolution: {},
        },
      },
    });
  };

  const handleRemove = (filename: string) => {
    if (!confirm(`Review a safe removal plan for "${filename}"?`)) return;
    setError(null);
    beginCanonicalOperation({ type: 'remove', filename });
  };

  const handleBulkRemove = (rows: InstalledContentRow[]): boolean => {
    const filenames = Array.from(new Set(rows.map((content) => content.filename)));
    if (filenames.length === 0) return false;
    const preview = filenames.length <= 3
      ? filenames.join(', ')
      : `${filenames.slice(0, 3).join(', ')} and ${filenames.length - 3} more`;
    if (!confirm(`Review one safe removal plan for ${filenames.length} selected item${filenames.length === 1 ? '' : 's'} (${preview})?`)) return false;
    setError(null);
    beginCanonicalOperation({ type: 'batch-remove', filenames });
    return true;
  };

  const updateLocalEnabled = (filename: string, enabled: boolean) => {
    setDetail((current) => {
      if (!current?.manifest) return current;
      return {
        ...current,
        manifest: updateManifestEntryEnabled(current.manifest, filename, enabled),
      };
    });
  };

  const dependentCandidates = (plans: DisablePlan[]) => {
    const candidates = new Map<string, { key: string; dependent: DependentInfo }>();
    plans.flatMap((plan) => plan.dependents).forEach((dependent) => {
      const key = `${dependent.filename}:${dependent.mod_id}`;
      if (!candidates.has(key)) candidates.set(key, { key, dependent });
    });
    return Array.from(candidates.values());
  };

  const handleToggleMod = async (mod: InstalledContentRow): Promise<boolean> => {
    setError(null);
    const enabled = !mod.enabled;
    if (mod.enabled && mod.content_type === 'mod') {
      const plan = await getDisablePlan(instanceId, mod.filename);
      if (plan.dependents.length > 0) {
        setDisablePlanTarget({ rows: [mod], candidates: dependentCandidates([plan]) });
        return false;
      }
    }
    if (mod.enabled) {
      await disableInstanceMod(instanceId, mod.filename);
    } else {
      await enableInstanceMod(instanceId, mod.filename);
    }
    updateLocalEnabled(mod.filename, enabled);
    await refreshContent();
    return true;
  };

  const handleBulkToggle = async (rows: InstalledContentRow[], enabled: boolean): Promise<boolean> => {
    setError(null);
    const targets = rows.filter((row) => row.enabled !== enabled);
    if (targets.length === 0) return true;
    if (!enabled) {
      const plans: DisablePlan[] = [];
      for (const target of targets.filter((row) => row.content_type === 'mod')) {
        plans.push(await getDisablePlan(instanceId, target.filename));
      }
      if (plans.some((plan) => plan.dependents.length > 0)) {
        setDisablePlanTarget({ rows: targets, candidates: dependentCandidates(plans) });
        return false;
      }
    }
    try {
      for (const target of targets) {
        if (enabled) await enableInstanceMod(instanceId, target.filename);
        else await disableInstanceMod(instanceId, target.filename);
      }
    } catch (error) {
      await refreshDetail().catch(() => undefined);
      throw error;
    }
    await refreshDetail();
    return true;
  };

  const handleDisablePlanConfirm = async (selectedKeys: string[]) => {
    if (!disablePlanTarget) return;
    setError(null);
    const selected = new Set(selectedKeys);
    const dependentFilenames = disablePlanTarget.candidates
      .filter((candidate) => selected.has(candidate.key))
      .map((candidate) => candidate.dependent.filename);
    const filenames = Array.from(new Set([
      ...disablePlanTarget.rows.filter((row) => row.enabled).map((row) => row.filename),
      ...dependentFilenames,
    ]));
    try {
      for (const filename of filenames) {
        await disableInstanceMod(instanceId, filename);
      }
      await refreshDetail();
      setDisablePlanTarget(null);
    } catch (error) {
      setError(formatError(error));
    }
  };

  const handleApplyUpdate = (row: InstalledContentRow, update: UpdateInfo) => {
    if (row?.enabled === false || !row.mod_jar_id && !row.modrinth_id && !row.registry_id) return;
    beginCanonicalOperation({
      type: 'batch-update',
      items: [{ itemId: update.mod_jar_id, targetVersion: update.target_version }],
    });
  };

  const handleSetInstanceIcon = async () => {
    if (!row?.is_modpack || row.is_locked) return;
    setError(null);
    try {
      const sourcePath = await pickOpenFile('Choose modpack icon', ['png', 'jpg', 'jpeg', 'webp', 'gif']);
      if (!sourcePath) return;
      const icon = await setCustomInstanceIcon(instanceId, sourcePath);
      setInstanceCustomIcon(icon);
      await refreshDetail();
    } catch (e) {
      setError(formatError(e));
    }
  };

  const handleSetModIcon = async (mod: InstalledMod) => {
    if (row?.is_locked) return;
    setError(null);
    try {
      const sourcePath = await pickOpenFile('Choose mod icon', ['png', 'jpg', 'jpeg', 'webp', 'gif']);
      if (!sourcePath) return;
      const icon = await setCustomModIcon(instanceId, mod.filename, sourcePath);
      setModCustomIcons((current) => ({ ...current, [installedModKey(mod)]: icon }));
      await refreshDetail();
    } catch (e) {
      setError(formatError(e));
    }
  };

  const handleInstallPackMods = async () => {
    if (!packIdInput.trim()) return;
    const packId = packIdInput.trim();
    setError(null);

    let mods: PackModRow[];
    try {
      mods = await listPackMods(packId);
    } catch (e) {
      setError(formatError(e));
      return;
    }
    if (mods.length === 0) {
      setError(`No mods found for pack "${packId}".`);
      return;
    }

    setPackProgress(mods.map((mod) => ({ modId: mod.mod_id, status: 'pending' as const })));
    const items: BatchInstallItem[] = [];
    let resolutionFailed = false;

    for (let index = 0; index < mods.length; index += 1) {
      const mod = mods[index];
      setPackProgress((previous) =>
        previous?.map((progress, current) =>
          current === index ? { ...progress, status: 'installing' as const } : progress
        ) ?? previous
      );
      try {
        const page = await listModVersions(instanceId, mod.mod_id);
        const candidate =
          page.items.find((version) => version.version_compat === 'compatible')
          ?? page.items.find((version) => version.version_compat === 'major_match')
          ?? page.items[0];
        if (!candidate) throw new Error('No compatible verified version is available.');
        items.push({
          sourceType: 'curated',
          itemId: mod.mod_id,
          candidateVersion: candidate.version,
        });
        setPackProgress((previous) =>
          previous?.map((progress, current) =>
            current === index ? { ...progress, status: 'done' as const } : progress
          ) ?? previous
        );
      } catch (e) {
        resolutionFailed = true;
        setPackProgress((previous) =>
          previous?.map((progress, current) =>
            current === index
              ? { ...progress, status: 'failed' as const, error: formatError(e) }
              : progress
          ) ?? previous
        );
      }
    }

    if (resolutionFailed) {
      setError('The pack plan could not be resolved completely. No instance files were changed.');
      return;
    }

    setPackProgress(null);
    setPackInstallOpen(false);
    beginCanonicalOperation({ type: 'batch-install', items });
  };

  const handleDismissPackProgress = () => {
    setPackProgress(null);
    setPackInstallOpen(false);
    setPackIdInput('');
    setError(null);
    // Reload manifest
    getInstanceDetail(instanceId).then((result) => setDetail(result));
  };

  // Refresh detail (row + manifest) after lock/unlock/revert.
  const refreshDetail = async () => {
    const result = await getInstanceDetail(instanceId);
    setDetail(result);
    await refreshContent();
  };

  const handleUnlock = async () => {
    setError(null);
    try {
      await unlockInstance(instanceId);
      await refreshDetail();
    } catch (e) {
      setError(formatError(e));
    }
  };

  const handleLock = async () => {
    setError(null);
    try {
      await lockInstance(instanceId);
      await refreshDetail();
    } catch (e) {
      setError(formatError(e));
    }
  };

  const handleRename = async () => {
    const newName = window.prompt('Rename instance', row?.name ?? '');
    if (!newName || newName.trim() === '' || newName.trim() === row?.name) return;
    setError(null);
    try {
      await renameInstance(instanceId, newName.trim());
      await refreshDetail();
      setStatus(`Renamed to "${newName.trim()}".`);
    } catch (e) {
      setError(formatError(e));
    }
  };

  const handleRevert = async () => {
    if (!confirm('Revert to the snapshot taken when this instance was unlocked? This removes any mods you added since then.')) {
      return;
    }
    setError(null);
    try {
      await revertInstance(instanceId);
      await refreshDetail();
    } catch (e) {
      setError(formatError(e));
    }
  };

  const handleImportPack = async () => {
    setError(null);
    setStatus(null);
    const path = await pickOpenFile('Import Pack', ['mrpack', 'agora-pack.json', 'json']);
    if (path === null) return;
    startPackFile(path, path.split(/[\\/]/).pop() ?? 'Pack import');
    setStatus('Pack import started in the background.');
  };

  const handleImportMod = async () => {
    setError(null);
    setStatus(null);
    const path = await pickOpenFile('Import Mod', ['jar']);
    if (path === null) return;
    beginCanonicalOperation({
      type: 'install',
      sourceType: 'manual',
      itemId: path.split(/[\\/]/).pop() ?? 'manual-mod',
      candidateVersion: path,
    });
  };

  const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setError(null);
    setStatus(null);
    const files = e.dataTransfer.files;
    if (!files || files.length === 0) return;
    const file = files[0];
    const filePath = (file as File & { path?: string }).path;
    if (!filePath) {
      setError('Could not resolve the dropped file path.');
      return;
    }
    try {
      const ext = file.name.toLowerCase();
      // .jar → manual mod install
      if (ext.endsWith('.jar')) {
        beginCanonicalOperation({
          type: 'install',
          sourceType: 'manual',
          itemId: file.name,
          candidateVersion: filePath,
        });
      }
      // .mrpack, .agora-pack.json, or .json → pack import
      else if (ext.endsWith('.mrpack') || ext.endsWith('.agora-pack.json') || (ext.endsWith('.json') && file.name.toLowerCase().endsWith('.json'))) {
        startPackFile(filePath, file.name);
        setStatus('Pack import started in the background.');
      }
      else {
        setError('Unsupported file type. Drop a .jar mod or a .mrpack/.agora-pack.json pack.');
      }
    } catch (e) {
      setError(formatError(e));
    }
  };

  const handleExportPack = async (format: 'json' | 'mrpack') => {
    setExportBusy(true);
    setError(null);
    setStatus(null);
    try {
      const path = await exportInstancePack(instanceId, format);
      setStatus(`Exported ${format === 'json' ? 'pack' : '.mrpack'} to: ${path}`);
      await revealPath(path);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setExportBusy(false);
    }
  };

  const requireLockfileText = () => {
    if (lockfileText.trim()) return lockfileText;
    setError('Export this instance or paste a lockfile before continuing.');
    return null;
  };

  const handleExportLockfile = async () => {
    setLockfileBusy('export');
    setError(null);
    setLockfileNotice(null);
    setLockfileReport(null);
    try {
      const lockfile = await exportLockfile(instanceId);
      setLockfileText(JSON.stringify(lockfile, null, 2));
      const artifacts = Array.isArray(lockfile.artifacts) ? lockfile.artifacts : [];
      const unresolved = artifacts.filter((artifact) => {
        if (!artifact || typeof artifact !== 'object') return false;
        return Boolean((artifact as Record<string, unknown>).unresolvedReason);
      }).length;
      setLockfileNotice(
        unresolved === 0
          ? 'Canonical lockfile exported. It contains hashes and settings, never private config contents.'
          : `Lockfile exported with ${unresolved} unreproducible artifact${unresolved === 1 ? '' : 's'} clearly marked. Verification still works, but clone and repair will refuse substitution.`,
      );
    } catch (cause) {
      setError(formatError(cause));
    } finally {
      setLockfileBusy(null);
    }
  };

  const handleCopyLockfile = async () => {
    const text = requireLockfileText();
    if (!text) return;
    setLockfileBusy('copy');
    setError(null);
    try {
      await navigator.clipboard.writeText(text);
      setLockfileNotice('Lockfile copied to the clipboard.');
    } catch (cause) {
      setError(`Could not copy the lockfile: ${formatError(cause)}`);
    } finally {
      setLockfileBusy(null);
    }
  };

  const handleVerifyLockfile = async () => {
    const text = requireLockfileText();
    if (!text) return;
    setLockfileBusy('verify');
    setError(null);
    setLockfileNotice(null);
    try {
      const report = await verifyLockfile(instanceId, text);
      setLockfileReport(report);
      setLockfileNotice(
        report.status === 'in-sync'
          ? 'This instance exactly matches the lockfile artifacts and tracked config hash.'
          : `${report.differences.length} difference${report.differences.length === 1 ? '' : 's'} found. Review them before repairing.`,
      );
    } catch (cause) {
      setLockfileReport(null);
      setError(formatError(cause));
    } finally {
      setLockfileBusy(null);
    }
  };

  const handleRepairLockfile = async () => {
    const text = requireLockfileText();
    if (!text) return;
    if (!window.confirm(
      'Repair this instance to the pasted lockfile? Agora will create one recovery snapshot, download exact hashes, and remove managed artifacts that are not in the lockfile. Private config contents cannot be repaired because lockfiles never contain them.',
    )) return;

    setLockfileBusy('repair');
    setError(null);
    setLockfileNotice(null);
    try {
      const outcome = await repairLockfile(instanceId, text);
      if (outcome.type === 'success') {
        await refreshDetail();
        const report = await verifyLockfile(instanceId, text);
        setLockfileReport(report);
        setLockfileNotice(
          report.status === 'in-sync'
            ? 'Repair completed and the instance now matches the lockfile.'
            : 'Artifact repair completed. Remaining differences cannot be reproduced from this privacy-preserving lockfile (usually private config changes).',
        );
      } else if (outcome.type === 'health-rollback') {
        setLockfileReport(null);
        setError('Repair introduced a health blocker, so Agora restored the recovery snapshot.');
      } else if (outcome.type === 'cancelled') {
        setLockfileReport(null);
        setLockfileNotice(
          outcome.rollbackPerformed
            ? 'Repair was cancelled and the recovery snapshot was restored.'
            : 'Repair was cancelled before the instance changed.',
        );
      } else {
        setLockfileReport(null);
        setError(
          outcome.rollbackPerformed
            ? `${outcome.error} The recovery snapshot was restored.`
            : outcome.error,
        );
      }
    } catch (cause) {
      setLockfileReport(null);
      setError(formatError(cause));
    } finally {
      setLockfileBusy(null);
    }
  };

  const handleCloneLockfile = async () => {
    const text = requireLockfileText();
    if (!text) return;
    setLockfileBusy('clone');
    setError(null);
    setLockfileNotice(null);
    try {
      const newInstanceId = await importLockfile(text);
      if (onOpenInstanceEditor) {
        onOpenInstanceEditor(newInstanceId);
      } else {
        setLockfileNotice(`Reproduced the lockfile as new instance "${newInstanceId}".`);
      }
    } catch (cause) {
      setError(formatError(cause));
    } finally {
      setLockfileBusy(null);
    }
  };

  const row = detail?.row;
  const manifest = detail?.manifest;
  const mods = manifest?.mods ?? [];
  const displayedContentRows = contentRowsLoaded ? contentRows : fallbackContentRows(manifest ?? null);
  const packInstall = getTaskForInstance(instanceId);
  const recoveryBlocked = (
    detail?.snapshot_readiness !== undefined && detail.snapshot_readiness !== 'ready'
  ) || packInstall?.status === 'running';
  const isCurrentProcess = processState?.instanceId === instanceId;
  const processLaunching = isCurrentProcess && processState?.phase === 'launching';
  const processStopping = isCurrentProcess && processState?.phase === 'stopping';
  const processRunning = isCurrentProcess && processState?.phase === 'running';
  const processDelegated = isCurrentProcess && processState?.phase === 'delegated';
  const anotherProcessActive = processState?.instanceId !== null
    && processState?.instanceId !== undefined
    && ['launching', 'running', 'stopping', 'delegated'].includes(processState.phase)
    && !isCurrentProcess;
  const playDisabled = playBusy || recoveryBlocked || processLaunching || processStopping || processDelegated || anotherProcessActive;
  const recoveryPending = detail?.snapshot_readiness === 'pending';
  const snapshotOperationPending = recoveryPending || packInstall?.status === 'running';
  const healthIssueCount = healthReport
    ? healthReport.blockers.length + healthReport.warnings.length
    : 0;
  const healthBlocked = (healthReport?.blockers.length ?? 0) > 0;

  useEffect(() => {
    if (packInstall?.status === 'failed' && packInstall.error) {
      setError(packInstall.error);
    }
  }, [packInstall?.error, packInstall?.status]);

  useEffect(() => {
    if (detail?.snapshot_readiness === 'failed' && detail.snapshot_error) {
      setError(detail.snapshot_error);
    }
  }, [detail?.snapshot_error, detail?.snapshot_readiness]);

  const handleOpenInstalledMod = (mod: InstalledContentRow | InstalledMod) => {
    const itemId = mod.registry_id || mod.modrinth_id || mod.mod_jar_id;
    if (itemId) onOpenModDetail?.(itemId);
  };

  const handleRevealInstalledContent = (content: InstalledContentRow) => {
    if (content.resolved_path) void revealPath(content.resolved_path).catch((cause) => setError(formatError(cause)));
  };

  if (loading) {
    return (
      <div className="space-y-6">
        <BackButton onBack={onBack} />
        <div className="rounded-xl border border-dashed border-border bg-card p-6 text-center text-muted-foreground">
          Loading instance…
        </div>
      </div>
    );
  }

  if (error && !row) {
    return (
      <div className="space-y-6">
        <BackButton onBack={onBack} />
        <div className="rounded-lg bg-destructive p-3 text-sm text-destructive-foreground">
          {error}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <BackButton onBack={onBack} />

      {/* Header */}
      <section className="rounded-xl border border-border bg-card p-6">
        <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
          <div className="flex items-start gap-4">
            {(instanceCustomIcon || packIconUrl(manifest)) && (
              <img
                src={instanceCustomIcon ?? packIconUrl(manifest) ?? undefined}
                alt={`${row?.name ?? 'Modpack'} icon`}
                className="h-16 w-16 shrink-0 rounded-xl border border-border object-cover"
              />
            )}
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-3">
              <h2 className="text-2xl font-bold">
                {row?.name}
                {' '}
                <button
                  onClick={handleRename}
                  disabled={recoveryBlocked}
                  className="text-xs text-muted-foreground hover:text-foreground underline"
                >
                  Rename
                </button>
              </h2>

            </div>
            <p className="text-xs text-muted-foreground mt-1">
              MC {row?.minecraft_version} · {manifest?.loader} {manifest?.loader_version}
            </p>
            {healthReport && healthIssueCount > 0 && (
              <div
                className={`mt-3 flex max-w-md items-center justify-between gap-3 rounded-lg border px-3 py-2 text-xs ${
                  healthBlocked
                    ? 'border-destructive/60 bg-destructive/10'
                    : 'border-amber-500/60 bg-amber-500/10'
                }`}
                role="alert"
                aria-label={`${healthIssueCount} health issue${healthIssueCount === 1 ? '' : 's'} detected`}
              >
                <div className="min-w-0">
                  <p className={healthBlocked ? 'font-medium text-destructive' : 'font-medium text-amber-700 dark:text-amber-300'}>
                    {healthIssueCount} health issue{healthIssueCount === 1 ? '' : 's'} detected
                  </p>
                  <p className="truncate text-muted-foreground">
                    {healthBlocked
                      ? 'Review before launching. You can continue after confirming risk.'
                      : 'Review recommended before launch.'}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => onReviewHealth?.(instanceId, row?.name ?? 'Instance', healthReport)}
                  className="shrink-0 rounded border border-current/30 px-2 py-1 font-medium hover:bg-background/40"
                >
                  Review & repair
                </button>
              </div>
            )}
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <span>{row?.is_locked ? '🔒 Locked' : '🔓 Unlocked'}</span>
              {row?.is_locked ? (
                <button
                  onClick={handleUnlock}
                  disabled={recoveryBlocked}
                  className="rounded-lg border border-input bg-background px-2.5 py-1 text-xs font-medium hover:bg-accent"
                >
                  Unlock
                </button>
              ) : (
                <>
                  <button
                    onClick={handleLock}
                    disabled={recoveryBlocked}
                    className="rounded-lg border border-input bg-background px-2.5 py-1 text-xs font-medium hover:bg-accent"
                  >
                    Lock
                  </button>
                  <button
                    onClick={handleRevert}
                    disabled={recoveryBlocked}
                    className="rounded-lg border border-input bg-background px-2.5 py-1 text-xs font-medium hover:bg-accent"
                  >
                    Revert
                  </button>
                </>
              )}
              {row?.last_launched_at && (
                <span className="ml-2">· Last launched {formatInstalledDate(row.last_launched_at)}</span>
              )}
              {row?.is_modpack && !row.is_locked && (
                <button
                  type="button"
                  onClick={handleSetInstanceIcon}
                  disabled={recoveryBlocked}
                  className="inline-flex items-center gap-1 rounded-lg border border-input bg-background px-2.5 py-1 text-xs font-medium hover:bg-accent"
                  title="Set a custom modpack image"
                >
                  <ImagePlus className="h-3.5 w-3.5" aria-hidden="true" />
                  Set image
                </button>
              )}
              </div>
            </div>
          </div>
          <div className="flex flex-col items-end gap-3 self-end xl:self-end">
            <div className="flex flex-wrap justify-end gap-2">
            <button
              onClick={() => {
                setPackInstallOpen(true);
                setPackIdInput('');
                setPackProgress(null);
                setError(null);
              }}
              disabled={recoveryBlocked}
              className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium"
            >
              📦 Install all mods from pack
            </button>
            <button
              onClick={handleImportPack}
              disabled={recoveryBlocked}
              className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium"
            >
              📥 Import Pack
            </button>
            <button
              onClick={() => openInstanceFolder(instanceId)}
              className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium"
              title="Open instance folder in file explorer"
            >
              📂 Open in Folder
            </button>
            <button
              onClick={() => onInvestigate?.(instanceId)}
              className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium"
              title="Analyze the latest instance logs"
            >
              Investigate
            </button>
            </div>
            {processRunning ? (
              <button
                type="button"
                onClick={() => { void onKillProcess?.(); }}
                disabled={!onKillProcess || processStopping}
                className="inline-flex items-center gap-2 rounded-lg bg-destructive px-5 py-3 text-base font-semibold text-destructive-foreground shadow-sm hover:bg-destructive/90 disabled:cursor-not-allowed disabled:opacity-50"
                aria-label={`Kill ${row?.name ?? 'instance'}`}
              >
                Kill
              </button>
            ) : (
              <button
                type="button"
                onClick={async () => {
                  if (!onLaunch || playDisabled) return;
                  setPlayBusy(true);
                  setError(null);
                  try {
                    await onLaunch(instanceId);
                  } catch (cause) {
                    setError(formatError(cause));
                  } finally {
                    setPlayBusy(false);
                  }
                }}
                disabled={!onLaunch || playDisabled}
                className="inline-flex items-center gap-2 rounded-lg bg-primary px-5 py-3 text-base font-semibold text-primary-foreground shadow-sm hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
                aria-label={`Play ${row?.name ?? 'instance'}`}
              >
                <Play className="h-5 w-5 fill-current" aria-hidden="true" />
                {processLaunching || playBusy ? 'Starting…' : processStopping ? 'Stopping…' : processDelegated ? 'Running via Mojang' : anotherProcessActive ? 'Game already running' : 'Play'}
              </button>
            )}
          </div>
        </div>

        {packInstall && <PackInstallProgressBar task={packInstall} />}

        {detail?.snapshot_readiness === 'pending' && (
          <div className="mt-4 rounded-lg border border-amber-500 bg-amber-500/10 p-3 text-sm" role="status">
            <p className="font-medium text-amber-700 dark:text-amber-300">Finalizing recovery snapshot…</p>
            <p className="mt-1 text-xs text-muted-foreground">
              You can inspect this instance while the snapshot builds. Launching and changes are temporarily disabled.
            </p>
          </div>
        )}
        {detail?.snapshot_readiness === 'failed' && (
          <div className="mt-4 rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm" role="alert">
            <p className="font-medium text-destructive">Recovery snapshot failed</p>
            <p className="mt-1 text-xs text-muted-foreground">{detail.snapshot_error ?? 'Create a new snapshot before changing or launching this instance.'}</p>
            <button
              type="button"
              onClick={async () => {
                try {
                  await createSnapshot(instanceId, 'Initial import retry');
                  setDetail(await getInstanceDetail(instanceId));
                  setStatus('Recovery snapshot ready.');
                } catch (cause) {
                  setError(formatError(cause));
                }
              }}
              className="mt-2 rounded-lg border border-destructive/50 px-3 py-1.5 text-xs font-medium hover:bg-destructive/10"
            >
              Retry recovery snapshot
            </button>
          </div>
        )}

        {error && (
          <div className="mt-4 rounded-lg bg-destructive p-3 text-sm text-destructive-foreground">
            {error}
          </div>
        )}
        {status && (
          <div className="mt-4 rounded-lg bg-accent text-accent-foreground p-3 text-sm">
            {status}
          </div>
        )}
      </section>

      {/* Sub-sidebar tabs */}
      <div className="flex border-b border-border gap-0">
        {(['mods', 'resourcepacks', 'shaders', 'datapacks', 'snapshots', 'loadout-profiles', 'import', 'export', 'console'] as const).map((tab) => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            className={[
              'px-4 py-2 text-sm font-medium border-b-2 transition-colors -mb-px',
              activeTab === tab
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground',
            ].join(' ')}
          >
            {tab === 'mods' ? `Mods (${mods.length})` : tab === 'resourcepacks' ? `Resource Packs (${manifest?.resourcepacks?.length ?? 0})` : tab === 'shaders' ? `Shaders (${manifest?.shaders?.length ?? 0})` : tab === 'datapacks' ? `Data Packs (${manifest?.datapacks?.length ?? 0})` : tab === 'snapshots' ? 'Snapshots' : tab === 'loadout-profiles' ? 'Loadout Profiles' : tab === 'import' ? 'Import' : tab === 'export' ? 'Export' : 'Console'}
          </button>
        ))}
        <button
            key="java-args"
            onClick={() => setActiveTab('java-args')}
            className={[
              'px-4 py-2 text-sm font-medium border-b-2 transition-colors -mb-px',
              activeTab === 'java-args'
                ? 'border-primary text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground',
            ].join(' ')}
          >
            Java & Args
        </button>
      </div>

      {activeTab === 'mods' && (
        <InstalledContentPanel
          contentType="mod"
          rows={displayedContentRows.filter((content) => content.content_type === 'mod')}
          locked={!!row?.is_locked || recoveryBlocked}
          addLabel="Import Mod"
          onAdd={handleImportMod}
          onToggle={handleToggleMod}
          onBulkToggle={handleBulkToggle}
          onBulkRemove={handleBulkRemove}
          onRemove={(content) => handleRemove(content.filename)}
          onOpenDetails={handleOpenInstalledMod}
          onRevealFile={handleRevealInstalledContent}
          onCheckUpdates={() => checkInstanceUpdates(instanceId)}
          onApplyUpdate={handleApplyUpdate}
          onSetCustomIcon={(content) => {
            const mod = mods.find((entry) => entry.filename === content.filename);
            if (mod) void handleSetModIcon(mod);
          }}
          onError={setError}
          onDrop={handleDrop}
          extraActions={<button type="button" onClick={() => onOpenBrowseForInstance?.(instanceId)} disabled={!!row?.is_locked || recoveryBlocked} className="rounded-lg border border-dashed border-border px-3 py-1.5 text-sm font-medium text-muted-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50" title={recoveryBlocked ? 'Wait for the recovery snapshot to finish.' : row?.is_locked ? 'Unlock the instance to add mods.' : undefined}>+ Add Mod</button>}
          iconForRow={(content) => {
            const mod = mods.find((entry) => entry.filename === content.filename);
            return mod ? modCustomIcons[installedModKey(mod)] ?? null : null;
          }}
        />
      )}

      {activeTab === 'resourcepacks' && (
        <InstalledContentPanel contentType="resourcepack" rows={displayedContentRows.filter((content) => content.content_type === 'resourcepack')} locked={!!row?.is_locked || recoveryBlocked} addLabel="+ Add Resource Pack" onAdd={() => onOpenBrowseForInstance?.(instanceId, 'resourcepack')} onToggle={handleToggleMod} onBulkToggle={handleBulkToggle} onBulkRemove={handleBulkRemove} onRemove={(content) => handleRemove(content.filename)} onOpenDetails={handleOpenInstalledMod} onRevealFile={handleRevealInstalledContent} onCheckUpdates={() => checkInstanceUpdates(instanceId)} onApplyUpdate={handleApplyUpdate} onError={setError} />
      )}

      {activeTab === 'shaders' && (
        <InstalledContentPanel contentType="shader" rows={displayedContentRows.filter((content) => content.content_type === 'shader')} locked={!!row?.is_locked || recoveryBlocked} addLabel="+ Add Shader" onAdd={() => onOpenBrowseForInstance?.(instanceId, 'shader')} onToggle={handleToggleMod} onBulkToggle={handleBulkToggle} onBulkRemove={handleBulkRemove} onRemove={(content) => handleRemove(content.filename)} onOpenDetails={handleOpenInstalledMod} onRevealFile={handleRevealInstalledContent} onCheckUpdates={() => checkInstanceUpdates(instanceId)} onApplyUpdate={handleApplyUpdate} onError={setError} />
      )}

      {activeTab === 'datapacks' && (
        <InstalledContentPanel contentType="datapack" rows={displayedContentRows.filter((content) => content.content_type === 'datapack')} locked={!!row?.is_locked || recoveryBlocked} addLabel="+ Add Data Pack" onAdd={() => onOpenBrowseForInstance?.(instanceId, 'datapack')} onToggle={handleToggleMod} onBulkToggle={handleBulkToggle} onBulkRemove={handleBulkRemove} onRemove={(content) => handleRemove(content.filename)} onOpenDetails={handleOpenInstalledMod} onRevealFile={handleRevealInstalledContent} onCheckUpdates={() => checkInstanceUpdates(instanceId)} onApplyUpdate={handleApplyUpdate} onError={setError} />
      )}

      {activeTab === 'mods' && (
        <>
      {/* Pack install progress */}
      {packInstallOpen && (
        <section className="rounded-xl border border-border bg-card p-4 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-sm">Install mods from pack</h3>
            {!packProgress && (
              <button
                onClick={() => {
                  setPackInstallOpen(false);
                  setPackIdInput('');
                  setError(null);
                }}
                className="text-xs text-muted-foreground hover:text-foreground"
              >
                Close
              </button>
            )}
          </div>

          {!packProgress ? (
            <div className="flex gap-2 relative" ref={packDropdownRef}>
              <div className="flex-1 relative">
                <input
                  type="text"
                  value={packIdInput}
                  disabled={recoveryBlocked}
                  onChange={(e) => {
                    setPackIdInput(e.target.value);
                    setPackDropdownOpen(true);
                  }}
                  onFocus={() => setPackDropdownOpen(true)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') handleInstallPackMods();
                    if (e.key === 'Escape') setPackDropdownOpen(false);
                  }}
                  placeholder="Search packs…"
                  className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
                />
                {packDropdownOpen && availablePacks.length > 0 && (
                  <div className="absolute z-50 mt-1 w-full rounded-lg border border-border bg-card shadow-lg max-h-48 overflow-y-auto">
                    {availablePacks
                      .filter((p) =>
                        !packIdInput ||
                        p.id.toLowerCase().includes(packIdInput.toLowerCase()) ||
                        p.name.toLowerCase().includes(packIdInput.toLowerCase())
                      )
                      .slice(0, 50)
                      .map((p) => (
                        <button
                          key={p.id}
                          onClick={() => {
                            setPackIdInput(p.id);
                            setPackDropdownOpen(false);
                          }}
                          disabled={recoveryBlocked}
                          className="w-full text-left px-3 py-2 text-sm hover:bg-accent border-b border-border last:border-b-0"
                        >
                          <span className="font-medium">{p.name}</span>
                          <span className="text-muted-foreground ml-2 text-xs">({p.id})</span>
                        </button>
                      ))}
                  </div>
                )}
              </div>
              <button
                onClick={handleInstallPackMods}
                disabled={!packIdInput.trim() || recoveryBlocked}
                className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              >
                Start
              </button>
            </div>
          ) : (
            <>
              <p className="text-xs text-muted-foreground">
                Installing pack: {packIdInput} ({packProgress.length} mods)
              </p>
              <div className="space-y-1">
                {packProgress.map((p, idx) => {
                  const icon =
                    p.status === 'done'
                      ? '✓'
                      : p.status === 'failed'
                        ? '✗'
                        : p.status === 'installing'
                          ? '⏳'
                          : '○';
                  const statusText =
                    p.status === 'done'
                      ? 'installed'
                      : p.status === 'failed'
                        ? p.error ?? 'failed'
                        : p.status === 'installing'
                          ? 'installing…'
                          : 'pending';
                  const lineColor =
                    p.status === 'done'
                      ? 'text-green-600 dark:text-green-400'
                      : p.status === 'failed'
                        ? 'text-destructive'
                        : p.status === 'installing'
                          ? 'text-yellow-600 dark:text-yellow-400'
                          : 'text-muted-foreground';
                  return (
                    <div key={idx} className={`text-sm ${lineColor}`}>
                      <span className="inline-block w-5 text-center">{icon}</span>{' '}
                      <span className="font-medium">{p.modId}</span> — {statusText}
                    </div>
                  );
                })}
              </div>

              {/* Summary + Done */}
              {packProgress.every((p) => p.status === 'done' || p.status === 'failed') && (
                <div className="border-t border-border pt-3">
                  {(() => {
                    const done = packProgress.filter((p) => p.status === 'done').length;
                    const failed = packProgress.filter((p) => p.status === 'failed');
                    if (failed.length === 0) {
                      return <p className="text-sm text-green-600 dark:text-green-400">Installed {done} mod{done !== 1 ? 's' : ''} successfully.</p>;
                    }
                    return (
                      <>
                        <p className="text-sm text-yellow-600 dark:text-yellow-400">
                          Installed {done} of {packProgress.length} mods. {failed.length} failed:
                        </p>
                        <ul className="mt-1 text-xs text-destructive space-y-0.5">
                          {failed.map((f, idx) => (
                            <li key={idx}>• {f.modId}: {f.error}</li>
                          ))}
                        </ul>
                      </>
                    );
                  })()}
                  <button
                    onClick={handleDismissPackProgress}
                    className="mt-3 rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
                  >
                    Done
                  </button>
                </div>
              )}
            </>
          )}
        </section>
      )}

        </>
      )}

      {activeTab === 'snapshots' && (
        <section className="rounded-xl border border-border bg-card p-4 space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-sm">Snapshots</h3>
            <div className="flex gap-2">
              <input
                type="text"
                value={snapshotLabelInput}
                disabled={snapshotOperationPending}
                onChange={(e) => setSnapshotLabelInput(e.target.value)}
                placeholder="Optional label…"
                className="rounded-lg border border-input bg-background px-3 py-1.5 text-sm w-48"
              />
              <button
                onClick={async () => {
                  setError(null);
                  try {
                    const label = snapshotLabelInput.trim() || `Snapshot ${new Date().toLocaleString()}`;
                    await createSnapshot(instanceId, label);
                    const result = await listSnapshots(instanceId);
                    setSnapshots(result);
                    setSnapshotLabelInput('');
                  } catch (e) {
                    setError(formatError(e));
                  }
                }}
                disabled={snapshotOperationPending}
                className="rounded-lg bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 whitespace-nowrap"
              >
                Create Snapshot
              </button>
            </div>
          </div>

          {snapshots.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No snapshots yet. Create one to save a restore point.
            </p>
          ) : (
            <div className="space-y-2">
              {snapshots.map((snap) => (
                <div key={snap.id} className="flex items-center justify-between rounded-lg border border-border px-3 py-2 text-sm">
                  <div className="min-w-0 flex-1">
                    <span className="font-medium flex items-center gap-2">
                      <span>{snap.label}</span>
                      {snap.is_current_lkg && (
                        <span className="rounded-full bg-green-500/10 px-2 py-0.5 text-[10px] text-green-700 dark:text-green-300">Current LKG</span>
                      )}
                      {!snap.is_current_lkg && snap.is_lkg && (
                        <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] text-primary">Known good</span>
                      )}
                      {snap.is_pre_restore && (
                        <span className="rounded-full bg-amber-500/10 px-2 py-0.5 text-[10px] text-amber-700 dark:text-amber-300">Undo restore</span>
                      )}
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {snap.created_at} · {snap.file_count} file{snap.file_count !== 1 ? 's' : ''}
                    </span>
                  </div>
                  <div className="flex gap-2 ml-3">
                    <button
                      onClick={async () => {
                        setSnapshotBusy(snap.id);
                        setError(null);
                        try {
                          const diff = await detectDrift(instanceId, snap.id);
                          setSnapshotDiff({ snapshotId: snap.id, diff });
                        } catch (e) {
                          setError(formatError(e));
                        } finally {
                          setSnapshotBusy(null);
                        }
                      }}
                       disabled={snapshotBusy === snap.id}
                      className="text-xs text-primary hover:underline disabled:opacity-50"
                    >
                      Show diff
                    </button>
                    <button
                      onClick={async () => {
                        setSnapshotBusy(snap.id);
                        setError(null);
                        try {
                          await restoreSnapshot(instanceId, snap.id);
                          const result = await listSnapshots(instanceId);
                          setSnapshots(result);
                          setDetail(await getInstanceDetail(instanceId));
                          setStatus('Snapshot restored.');
                        } catch (e) {
                          setError(formatError(e));
                        } finally {
                          setSnapshotBusy(null);
                        }
                      }}
                       disabled={snapshotBusy === snap.id || snapshotOperationPending}
                      className="text-xs text-foreground hover:underline disabled:opacity-50"
                    >
                      {snapshotBusy === snap.id ? 'Restoring…' : 'Restore'}
                    </button>
                    {confirmDeleteSnapshot === snap.id ? (
                      <div className="flex gap-1">
                        <button
                          onClick={async () => {
                            setSnapshotBusy(snap.id);
                            setError(null);
                            try {
                              await deleteSnapshot(instanceId, snap.id);
                              const result = await listSnapshots(instanceId);
                              setSnapshots(result);
                              setConfirmDeleteSnapshot(null);
                            } catch (e) {
                              setError(formatError(e));
                            } finally {
                              setSnapshotBusy(null);
                            }
                          }}
                           disabled={snapshotOperationPending}
                           className="text-xs text-destructive font-medium disabled:opacity-50"
                        >
                          Confirm
                        </button>
                        <button
                         onClick={() => setConfirmDeleteSnapshot(null)}
                         disabled={snapshotOperationPending}
                          className="text-xs text-muted-foreground"
                        >
                          Cancel
                        </button>
                      </div>
                    ) : (
                      <button
                         onClick={() => setConfirmDeleteSnapshot(snap.id)}
                         disabled={snapshotOperationPending}
                         className="text-xs text-destructive hover:underline disabled:opacity-50"
                      >
                        Delete
                      </button>
                    )}
                  </div>
                </div>
              ))}
              {snapshotDiff && (
                <div className="rounded-lg border border-border bg-muted/30 p-3 text-xs space-y-2">
                  <div className="flex items-center justify-between gap-3">
                    <p className="font-semibold">Changes since snapshot</p>
                    <button onClick={() => setSnapshotDiff(null)} className="text-muted-foreground hover:text-foreground">Close</button>
                  </div>
                  <p className="text-muted-foreground">
                    +{snapshotDiff.diff.added.length} added · -{snapshotDiff.diff.removed.length} removed · ~{snapshotDiff.diff.modified.length} modified · {snapshotDiff.diff.unchangedCount} unchanged
                  </p>
                  {[
                    ...snapshotDiff.diff.added.map((entry) => ({ ...entry, marker: '+', label: 'Added' })),
                    ...snapshotDiff.diff.removed.map((entry) => ({ ...entry, marker: '-', label: 'Removed' })),
                    ...snapshotDiff.diff.modified.map((entry) => ({ ...entry, marker: '~', label: 'Modified' })),
                  ].length > 0 ? (
                    <ul className="max-h-48 overflow-y-auto space-y-1 font-mono">
                      {[
                        ...snapshotDiff.diff.added.map((entry) => ({ ...entry, marker: '+', label: 'Added' })),
                        ...snapshotDiff.diff.removed.map((entry) => ({ ...entry, marker: '-', label: 'Removed' })),
                        ...snapshotDiff.diff.modified.map((entry) => ({ ...entry, marker: '~', label: 'Modified' })),
                      ].map((entry) => (
                        <li key={`${entry.marker}:${entry.path}`} className="break-all">
                          <span className="font-semibold">{entry.marker}</span> {entry.path} <span className="font-sans text-muted-foreground">({entry.label})</span>
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <p className="text-green-700 dark:text-green-300">The current instance matches this snapshot exactly.</p>
                  )}
                </div>
              )}
            </div>
          )}
        </section>
      )}

      {activeTab === 'loadout-profiles' && (
        <section className="rounded-xl border border-border bg-card p-4 space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-sm">Loadout Profiles</h3>
            <div className="flex gap-2">
              <input
                type="text"
                value={profileNameInput}
                disabled={recoveryBlocked}
                onChange={(e) => setProfileNameInput(e.target.value)}
                placeholder="Profile name…"
                className="rounded-lg border border-input bg-background px-3 py-1.5 text-sm w-48"
              />
              <button
                onClick={async () => {
                  setError(null);
                  try {
                    const name = profileNameInput.trim() || `Current Setup ${new Date().toLocaleString()}`;
                    await createLoadoutProfile(instanceId, name);
                    const result = await listLoadoutProfiles(instanceId);
                    setProfiles(result);
                    setProfileNameInput('');
                    setStatus(`Profile "${name}" created.`);
                  } catch (e) {
                    setError(formatError(e));
                  }
                }}
                disabled={recoveryBlocked}
                className="rounded-lg bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 whitespace-nowrap"
              >
                Create Profile
              </button>
            </div>
          </div>

          {profiles.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No loadout profiles yet. Enter a name and click Create Profile.
            </p>
          ) : (
            <div className="space-y-2">
              {profiles.map((prof) => (
                <div key={prof.name} className="flex items-center justify-between rounded-lg border border-border px-3 py-2 text-sm">
                  <div className="min-w-0 flex-1">
                    <span className="font-medium block">{prof.name}</span>
                    <span className="text-xs text-muted-foreground">{prof.enabled_mods.length} mod{prof.enabled_mods.length !== 1 ? 's' : ''}</span>
                  </div>
                  <div className="flex gap-2 ml-3">
                    <button
                      onClick={async () => {
                        setProfileBusy(prof.name);
                        setError(null);
                        try {
                          await applyLoadoutProfile(instanceId, prof.name);
                          const result = await listLoadoutProfiles(instanceId);
                          setProfiles(result);
                          setDetail(await getInstanceDetail(instanceId));
                          setStatus(`Profile "${prof.name}" applied.`);
                        } catch (e) {
                          setError(formatError(e));
                        } finally {
                          setProfileBusy(null);
                        }
                      }}
                       disabled={profileBusy === prof.name || recoveryBlocked}
                      className="text-xs text-foreground hover:underline disabled:opacity-50"
                    >
                      {profileBusy === prof.name ? 'Applying…' : 'Apply'}
                    </button>
                    {confirmDeleteProfile === prof.name ? (
                      <div className="flex gap-1">
                        <button
                          onClick={async () => {
                            setProfileBusy(prof.name);
                            setError(null);
                            try {
                              await deleteLoadoutProfile(instanceId, prof.name);
                              const result = await listLoadoutProfiles(instanceId);
                              setProfiles(result);
                              setConfirmDeleteProfile(null);
                            } catch (e) {
                              setError(formatError(e));
                            } finally {
                              setProfileBusy(null);
                            }
                          }}
                           disabled={recoveryBlocked}
                           className="text-xs text-destructive font-medium disabled:opacity-50"
                        >
                          Confirm
                        </button>
                        <button
                         onClick={() => setConfirmDeleteProfile(null)}
                         disabled={recoveryBlocked}
                          className="text-xs text-muted-foreground"
                        >
                          Cancel
                        </button>
                      </div>
                    ) : (
                      <button
                         onClick={() => setConfirmDeleteProfile(prof.name)}
                         disabled={recoveryBlocked}
                         className="text-xs text-destructive hover:underline disabled:opacity-50"
                      >
                        Delete
                      </button>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </section>
      )}

      {activeTab === 'import' && (
        <section className="rounded-xl border border-border bg-card p-4 space-y-4">
          <h3 className="font-semibold text-sm">Import a new instance</h3>
          <p className="text-xs text-muted-foreground">
            Copy instances from Prism Launcher, CurseForge, or Modrinth App, or import a
            Modrinth pack (.mrpack) or ZIP archive from disk.
          </p>
          <button
            onClick={() => setLauncherImportOpen(true)}
            className="w-full rounded-lg border border-border px-4 py-2 text-sm font-medium hover:bg-accent"
          >
            Import from Installed Launchers
          </button>
          <button
            onClick={async () => {
              setImportBusy(true);
              setError(null);
              try {
                const path = await pickOpenFile('Import Instance', ['mrpack', 'zip']);
                if (path === null) { setImportBusy(false); return; }
                const result = await importInstance(path, false);
                if (onOpenInstanceEditor) {
                  onOpenInstanceEditor(result.instance_id);
                } else {
                  setStatus(`Imported "${result.name}" (MC ${result.minecraft_version}).`);
                }
              } catch (e) {
                setError(formatError(e));
              } finally {
                setImportBusy(false);
              }
            }}
            disabled={importBusy}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50 w-full"
          >
            {importBusy ? 'Importing…' : 'Select File & Import'}
          </button>
          <p className="text-xs text-muted-foreground">
            Agora always copies imported data. Source instances and saves are never linked or modified.
          </p>
        </section>
      )}

      <LauncherImportWizard
        open={launcherImportOpen}
        onClose={() => setLauncherImportOpen(false)}
        onComplete={() => setLauncherImportOpen(false)}
      />

      {activeTab === 'export' && (
        <section className="space-y-4">
          <div>
            <h3 className="font-semibold text-sm">Export Instance</h3>
            <p className="text-xs text-muted-foreground mt-1">
              Choose how to share or back up this instance. Each format serves a different
              purpose — pick the one that matches your goal.
            </p>
          </div>

          {/* Card 1: mrpack (Recommended) */}
          <div className="rounded-xl border border-border bg-card p-4 space-y-3">
            <div className="flex items-start justify-between gap-2">
              <h4 className="font-semibold text-sm">Export as Modrinth Pack (.mrpack)</h4>
              <span className="rounded-full bg-primary/10 text-primary text-[10px] font-semibold px-2 py-0.5 shrink-0">
                Recommended
              </span>
            </div>
            <p className="text-xs text-muted-foreground">
              The industry-standard format for sharing Minecraft modpacks. Compatible with
              Modrinth, Prism Launcher, and other launchers. Contains mod references, config
              files, and overrides — not the mod files themselves.
            </p>
            <p className="text-xs text-muted-foreground">
              <span className="font-medium text-foreground">Best for:</span> sharing your
              modpack with other launchers or publishing to Modrinth.
            </p>
            <button
              onClick={() => handleExportPack('mrpack')}
              disabled={exportBusy}
              className="rounded-lg bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
            >
              {exportBusy ? 'Exporting…' : 'Export .mrpack'}
            </button>
          </div>

          {/* Card 2: Agora JSON pack */}
          <div className="rounded-xl border border-border bg-card p-4 space-y-3">
            <div className="flex items-start justify-between gap-2">
              <h4 className="font-semibold text-sm">Export as Agora Pack (.json)</h4>
              <span className="rounded-full bg-muted text-muted-foreground text-[10px] font-semibold px-2 py-0.5 shrink-0">
                Agora native
              </span>
            </div>
            <p className="text-xs text-muted-foreground">
              Agora's native pack format. Contains the full mod list with exact versions and
              source references. Reimport into any Agora instance to recreate this loadout.
            </p>
            <p className="text-xs text-muted-foreground">
              <span className="font-medium text-foreground">Best for:</span> backing up your
              mod selection or sharing with other Agora users.
            </p>
            <button
              onClick={() => handleExportPack('json')}
              disabled={exportBusy}
              className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium disabled:opacity-50"
            >
              {exportBusy ? 'Exporting…' : 'Export agora-pack.json'}
            </button>
          </div>

          {/* Card 3: Lockfile (Advanced) */}
          <div className="rounded-xl border border-border bg-card p-4 space-y-3">
            <div className="flex items-start justify-between gap-2">
              <h4 className="font-semibold text-sm">Export Reproduction Lockfile</h4>
              <span className="rounded-full bg-muted text-muted-foreground text-[10px] font-semibold px-2 py-0.5 shrink-0">
                Advanced
              </span>
            </div>
            <p className="text-xs text-muted-foreground">
              A privacy-preserving lockfile recording SHA-256 hashes, exact download sources,
              mod versions, and all settings. Any installation with the same lockfile reproduces
              identical artifacts. Private config contents are never included.
            </p>
            <p className="text-xs text-muted-foreground">
              <span className="font-medium text-foreground">Best for:</span> forensic
              reproduction, drift detection, and bit-identical cloning.
            </p>

            <div className="flex flex-wrap gap-2">
              <button
                onClick={() => void handleExportLockfile()}
                disabled={lockfileBusy !== null}
                className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium disabled:opacity-50"
              >
                {lockfileBusy === 'export' ? 'Exporting…' : 'Export Lockfile'}
              </button>
              {lockfileText.trim() && (
                <>
                  <button
                    onClick={() => void handleCopyLockfile()}
                    disabled={lockfileBusy !== null}
                    className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium disabled:opacity-50"
                  >
                    {lockfileBusy === 'copy' ? 'Copying…' : 'Copy'}
                  </button>
                  <button
                    onClick={() => {
                      setLockfileText('');
                      setLockfileReport(null);
                      setLockfileNotice(null);
                      setError(null);
                    }}
                    disabled={lockfileBusy !== null}
                    className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium disabled:opacity-50"
                  >
                    Clear
                  </button>
                </>
              )}
            </div>

            <textarea
              value={lockfileText}
              onChange={(event) => {
                setLockfileText(event.target.value);
                setLockfileReport(null);
                setLockfileNotice(null);
                setError(null);
              }}
              rows={12}
              aria-label="Instance lockfile JSON"
              placeholder="Export this instance or paste an Agora lockfile JSON here…"
              className="w-full rounded-lg border border-input bg-background p-3 text-xs font-mono resize-y"
            />

            {lockfileText.trim() ? (
          <div className="flex flex-wrap justify-end gap-2">
                <button
                  onClick={() => void handleVerifyLockfile()}
                  disabled={lockfileBusy !== null}
                  className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium disabled:opacity-50"
                >
                  {lockfileBusy === 'verify' ? 'Verifying…' : 'Verify'}
                </button>
                <button
                  onClick={() => void handleRepairLockfile()}
                  disabled={lockfileBusy !== null || Boolean(row?.is_locked) || recoveryBlocked}
                  title={recoveryBlocked ? 'Wait for the recovery snapshot to finish.' : row?.is_locked ? 'Unlock this instance before repairing drift.' : undefined}
                  className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium disabled:opacity-50"
                >
                  {lockfileBusy === 'repair' ? 'Repairing…' : 'Repair'}
                </button>
                <button
                  onClick={() => void handleCloneLockfile()}
                  disabled={lockfileBusy !== null}
                  className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium disabled:opacity-50"
                >
                  {lockfileBusy === 'clone' ? 'Cloning…' : 'Clone'}
                </button>
              </div>
            ) : (
              <div className="rounded-lg border border-dashed border-border bg-muted p-4 text-center text-xs text-muted-foreground">
                Export this instance or paste a received lockfile to verify, repair, or clone it.
              </div>
            )}

            {lockfileNotice && (
              <div className="rounded-lg bg-accent/20 p-3 text-xs text-muted-foreground">{lockfileNotice}</div>
            )}

            {lockfileReport && (
              <div className="rounded-lg border border-border bg-background p-3 space-y-1 text-xs">
                <p className="font-medium">
                  {lockfileReport.status === 'in-sync' ? 'In sync' : 'Drift detected'}
                </p>
                {lockfileReport.differences?.map((diff, idx) => (
                  <p key={idx} className="text-muted-foreground">
                    {diff.path}: {diff.kind} {diff.expectedSha256 && `(expected ${diff.expectedSha256})`} {diff.actualSha256 && `(got ${diff.actualSha256})`}
                  </p>
                ))}
              </div>
            )}
          </div>
        </section>
      )}

      {activeTab === 'console' && (
        <section className="rounded-xl border border-border bg-card p-4 space-y-3">
          <h3 className="font-semibold text-sm">Game Console</h3>
          <p className="text-xs text-muted-foreground">
            Live stdout/stderr from the launched Minecraft process. Logs stream here when the instance is running via Agora's direct launcher.
          </p>
          <ConsoleView
            instanceId={instanceId}
            className="mt-2"
            logBuffer={processLogs?.filter((l) => l.instance_id === instanceId)}
          />
        </section>
      )}

      {activeTab === 'java-args' && (
        <section className="rounded-xl border border-border bg-card p-4 space-y-4">
          <h3 className="font-semibold text-sm">Java & Args</h3>
          <p className="text-xs text-muted-foreground">
            Configure per-instance Java runtime path. By default Agora auto-selects the exact
            major version required by the instance's Minecraft version. Override only when you
            need a specific Java distribution for this instance.
          </p>

          {/* Per-instance Java path */}
          <div className="space-y-2">
            <label className="text-sm font-medium">Java executable path</label>
            <p className="text-xs text-muted-foreground">
              Leave empty to use the global default (from Settings) or auto-detection.
            </p>
            <div className="flex gap-2">
              <input
                value={instanceJavaPath}
                disabled={recoveryBlocked}
                onChange={(e) => {
                  setInstanceJavaPath(e.target.value);
                  setInstanceJavaInspected(null);
                  setInstanceJavaInspectError(null);
                }}
                placeholder="Auto (global default)"
                className="flex-1 rounded-lg border border-input bg-background px-3 py-2 text-sm"
              />
              <button
                onClick={async () => {
                  setInstanceJavaInspectError(null);
                  setInstanceJavaInspected(null);
                  try {
                    const chosen = await pickOpenFile('Select Java executable', ['exe', 'java']);
                    if (chosen) {
                      setInstanceJavaPath(chosen);
                      const info = await inspectJavaExecutable(chosen);
                      setInstanceJavaInspected(info);
                    }
                  } catch (e) {
                    setInstanceJavaInspectError(formatError(e));
                  }
                }}
                disabled={recoveryBlocked}
                className="rounded-lg border border-input px-3 py-2 text-sm font-medium hover:bg-accent"
              >
                Browse…
              </button>
            </div>

            {/* Inspect result */}
            {instanceJavaInspected && (
              <div className="rounded-lg bg-muted px-3 py-2 space-y-1">
                <p className="text-xs text-green-600 dark:text-green-400">Java {instanceJavaInspected.version} detected</p>
                <p className="text-xs text-muted-foreground">
                  {instanceJavaInspected.version_string} · {instanceJavaInspected.arch ?? 'unknown arch'}
                </p>
                <p className="text-xs text-muted-foreground">
                  Source: <span className="font-medium">{instanceJavaInspected.source}</span>
                </p>
              </div>
            )}
            {instanceJavaInspectError && (
              <p className="text-xs text-destructive">{instanceJavaInspectError}</p>
            )}

            <div className="rounded-lg border border-border bg-background p-4 space-y-4">
              <div>
                <h4 className="text-sm font-semibold">Automatic JVM tuning</h4>
                <p className="mt-1 text-xs text-muted-foreground">
                  Agora keeps heap size, Java compatibility, and garbage collection flags together so you do not need to hand-build JVM arguments.
                </p>
              </div>

              <label className="block space-y-2">
                <span className="flex items-center justify-between text-sm font-medium">
                  <span>Memory allocation</span>
                  <span className="tabular-nums text-primary">
                    {instanceMemoryMode === 'auto'
                      ? `Auto - ${((memoryRecommendation?.recommended_mb ?? instanceJvmMemory) / 1024).toFixed(1)} GB`
                      : instanceJvmMemory >= 1024
                        ? `${(instanceJvmMemory / 1024).toFixed(instanceJvmMemory % 1024 === 0 ? 0 : 1)} GB`
                        : `${instanceJvmMemory} MB`}
                  </span>
                </span>
                <span className="flex items-center justify-between rounded-md border border-border px-3 py-2 text-xs">
                  <span>
                    <span className="block font-medium">Estimate from pack size</span>
                    <span className="block text-muted-foreground">Recalculates when enabled content changes.</span>
                  </span>
                  <input
                    aria-label="Automatic memory allocation"
                    type="checkbox"
                    checked={instanceMemoryMode === 'auto'}
                    disabled={recoveryBlocked}
                    onChange={(event) => setInstanceMemoryMode(event.target.checked ? 'auto' : 'manual')}
                  />
                </span>
                <input
                  aria-label="Memory allocation"
                  type="range"
                  min={2048}
                  max={32768}
                  step={512}
                  value={instanceJvmMemory}
                  disabled={recoveryBlocked || instanceMemoryMode === 'auto'}
                  onChange={(e) => setInstanceJvmMemory(Number(e.target.value))}
                  className="w-full accent-primary"
                />
                <span className="flex justify-between text-[11px] text-muted-foreground">
                  <span>2 GB</span>
                  <span>32 GB maximum</span>
                </span>
                {instanceMemoryMode === 'auto' && memoryRecommendation && (
                  <span className={memoryRecommendation.insufficient_system_ram ? 'text-xs text-destructive' : 'text-xs text-muted-foreground'}>
                    {memoryRecommendation.explanation}
                  </span>
                )}
              </label>

              <label className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
                <span>
                  <span className="block text-sm font-medium">Choose the best GC automatically</span>
                  <span className="block text-xs text-muted-foreground">Uses Generational ZGC on Java 21+ and tuned G1GC on older Java.</span>
                </span>
                <input
                  aria-label="Automatic GC selection"
                  type="checkbox"
                  checked={instanceGcMode === 'auto'}
                  disabled={recoveryBlocked}
                  onChange={(e) => setInstanceGcMode(e.target.checked ? 'auto' : 'high_efficiency')}
                  className="h-5 w-5 shrink-0 accent-primary"
                />
              </label>

              {instanceGcMode !== 'auto' && (
                <label className="block space-y-2">
                  <span className="text-sm font-medium">Garbage collector</span>
                  <select
                    aria-label="Garbage collector"
                    value={instanceGcMode}
                    disabled={recoveryBlocked}
                    onChange={(e) => setInstanceGcMode(e.target.value as GcMode)}
                    className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
                  >
                    <option value="high_efficiency">G1GC · high efficiency</option>
                    <option value="low_latency">ZGC · low latency (Java 15+)</option>
                    {advancedMode && <option value="manual">Manual flags</option>}
                  </select>
                </label>
              )}

              <label className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
                <span>
                  <span className="block text-sm font-medium">Pre-touch allocated memory</span>
                  <span className="block text-xs text-muted-foreground">Reduces in-game stutter at the cost of a longer startup. Recommended for G1GC.</span>
                </span>
                <input
                  aria-label="Pre-touch allocated memory"
                  type="checkbox"
                  checked={instanceAlwaysPreTouch}
                  disabled={recoveryBlocked}
                  onChange={(e) => setInstanceAlwaysPreTouch(e.target.checked)}
                  className="h-5 w-5 shrink-0 accent-primary"
                />
              </label>

              <div className="rounded-lg bg-muted px-3 py-2 text-xs">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium">Launch preview</span>
                  {gcPreviewLoading && <span className="text-muted-foreground">Updating…</span>}
                </div>
                <p className="mt-1 text-muted-foreground">
                  Selected: {instanceGcMode === 'auto' ? 'Auto' : instanceGcMode === 'manual' ? 'Manual' : instanceGcMode === 'low_latency' ? 'ZGC low latency' : 'G1GC high efficiency'}
                </p>
                {gcPreview ? (
                  <>
                    <p className="mt-1 text-muted-foreground">
                      {gcPreview.profile === 'low_latency' ? 'Generational ZGC' : gcPreview.profile === 'high_efficiency' ? 'Tuned G1GC' : 'Manual JVM flags'} · {gcPreview.heap_mb >= 1024 ? `${(gcPreview.heap_mb / 1024).toFixed(1)} GB effective heap` : `${gcPreview.heap_mb} MB effective heap`}
                    </p>
                    <code className="mt-2 block max-h-20 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] text-foreground/80">{gcPreview.jvm_args}</code>
                    {gcPreview.heap_mb !== instanceJvmMemory && (
                      <p className="mt-1 text-amber-700 dark:text-amber-400">The launch-time safety limit adjusted the heap to leave room for the OS.</p>
                    )}
                  </>
                ) : (
                  <p className="mt-1 text-muted-foreground">Launch preview unavailable until the backend responds.</p>
                )}
              </div>

              {/* Manual flags remain available without making them the default workflow. */}
              {advancedMode && instanceGcMode === 'manual' && (
                <div className="space-y-2">
                  <label htmlFor="instance-java-args" className="text-sm font-medium">Additional JVM flags</label>
                  <p className="text-xs text-muted-foreground">
                    Advanced flags are appended after Agora&apos;s managed memory settings. Do not include classpath or native-library flags.
                  </p>
                  <textarea
                    id="instance-java-args"
                    value={instanceJavaArgs}
                    disabled={recoveryBlocked}
                    onChange={(e) => setInstanceJavaArgs(e.target.value)}
                    rows={4}
                    placeholder="-Xss1M -Dsome.setting=true"
                    className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm font-mono resize-y"
                  />
                </div>
              )}

            {/* Allow incompatible override (Advanced Mode only) */}
            {advancedMode && (
              <div className="space-y-2">
                <label className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={instanceJavaAllowOverride}
                    disabled={recoveryBlocked}
                    onChange={(e) => setInstanceJavaAllowOverride(e.target.checked)}
                    className="h-4 w-4 accent-primary"
                  />
                  <span className="text-sm">
                    Allow this Java version even when Minecraft requests a different version (⚠ advanced users only)
                  </span>
                </label>
                {instanceJavaAllowOverride && (
                  <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2">
                    <p className="text-xs text-amber-700 dark:text-amber-400 font-medium">⚠ Compatibility warning</p>
                    <p className="text-xs text-amber-600/80 dark:text-amber-400/80 mt-0.5">
                      Using an incompatible Java version may cause crashes or unexpected behavior.
                      Only enable this if you understand the risks and have verified that the
                      selected Java runtime works with this Minecraft version.
                    </p>
                  </div>
                )}
              </div>
            )}

            {/* Save / Clear buttons */}
            <div className="flex gap-2">
              <button
                onClick={async () => {
                  setInstanceJavaSaving(true);
                  setInstanceJavaInspectError(null);
                  try {
                    // Validate path if non-empty
                    if (instanceJavaPath.trim()) {
                      await inspectJavaExecutable(instanceJavaPath.trim());
                    }
                    await updateInstanceJava(
                      instanceId,
                      instanceJavaPath.trim() || null,
                      instanceJavaAllowOverride,
                    );
                    await updateInstanceJvm(
                      instanceId,
                      instanceJvmMemory,
                      instanceGcMode,
                      instanceAlwaysPreTouch,
                      instanceJavaArgs.trim(),
                      instanceMemoryMode,
                    );
                    setStatus('Java settings saved.');
                    // Refresh to update the displayed detail
                    const fresh = await getInstanceDetail(instanceId);
                    setDetail(fresh);
                  } catch (e) {
                    setInstanceJavaInspectError(formatError(e));
                  } finally {
                    setInstanceJavaSaving(false);
                  }
                }}
                disabled={instanceJavaSaving || recoveryBlocked}
                className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              >
                {instanceJavaSaving ? 'Saving…' : 'Save'}
              </button>
              <button
                onClick={async () => {
                  setInstanceJavaPath('');
                  setInstanceJavaArgs('');
                  setInstanceJavaInspected(null);
                  setInstanceJavaInspectError(null);
                  try {
                     await updateInstanceJava(instanceId, null, false);
                     await updateInstanceJvm(
                       instanceId,
                       instanceJvmMemory,
                       instanceGcMode,
                        instanceAlwaysPreTouch,
                        '',
                        instanceMemoryMode,
                     );
                    setStatus('Java settings cleared.');
                    const fresh = await getInstanceDetail(instanceId);
                    setDetail(fresh);
                  } catch (e) {
                    setInstanceJavaInspectError(formatError(e));
                  }
                }}
                disabled={recoveryBlocked}
                className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent disabled:opacity-50"
              >
                Clear
              </button>
            </div>
          </div>
          </div>
        </section>
      )}

      {disablePlanTarget && (
        <DependencyPrompt
          title={disablePlanTarget.rows.length > 1 ? 'Disable selected content and dependents' : 'Disable content and dependents'}
          description="The selected content is required by other installed mods. Review the affected rows before disabling anything."
          actionLabel="Disable selected"
          candidates={disablePlanTarget.candidates.map(({ key, dependent }) => ({
            key,
            label: dependent.filename || dependent.mod_id,
            requirement: dependent.requirement,
            source: dependent.source,
          }))}
          onConfirm={handleDisablePlanConfirm}
          onCancel={() => setDisablePlanTarget(null)}
        />
      )}

      {canonicalOperation && (
        <InstallFlow
          open
          intent={canonicalOperation.intent}
          instanceName={canonicalOperation.instanceName}
          background
          onBackgroundStart={(plan) => startPlan(plan, `Installing pack in ${canonicalOperation.instanceName}`, canonicalOperation.instanceName)}
          onOpenInstance={onOpenInstanceEditor}
           onClose={() => {
             setCanonicalOperation(null);
             void getInstanceDetail(instanceId)
               .then((result) => {
                 setDetail(result);
                 return refreshContent();
               })
               .catch((cause) => setError(formatError(cause)));
           }}
        />
      )}
    </div>
  );
}

function BackButton({ onBack }: { onBack: () => void }) {
  return (
    <button
      onClick={onBack}
      className="rounded-lg border border-input bg-background hover:bg-accent px-3 py-1.5 text-sm font-medium"
    >
      ← Back
    </button>
  );
}
