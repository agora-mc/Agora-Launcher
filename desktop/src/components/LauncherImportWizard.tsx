import { useState, useCallback, useEffect, useRef, useMemo } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  Dialog,
  DialogContent,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Switch } from '@/components/ui/switch';
import { cn } from '@/lib/utils';
import {
  discoverLauncherImports,
  planLauncherImports,
  executeLauncherImports,
  pickDirectory,
  cancelOperation,
  formatError,
} from '@/lib/tauri';
import type {
  LauncherImportDiscovery,
  LauncherImportCandidate,
  LauncherImportPlan,
  ImportSelection,
  CandidateStatus,
  LauncherDiscovery,
} from '@/lib/tauri';
import {
  Search,
  FolderPlus,
  CheckCircle2,
  XCircle,
  Package,
  Image,
  Sun,
  Database,
  Save,
  Layers,
  Flame,
  Gem,
  ArrowLeft,
  Loader2,
  AlertTriangle,
  Info,
  Ban,
} from 'lucide-react';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type Stage = 'detect' | 'select' | 'review' | 'importing' | 'results';

interface LauncherGroup {
  kind: 'prism' | 'curse_forge' | 'modrinth';
  label: string;
  icon: React.ReactNode;
  detection: LauncherDiscovery;
  candidates: LauncherImportCandidate[];
}

interface OperationProgressPayload {
  operationId: string;
  phase: string;
  message: string;
  progress: number;
  subLabel: string;
  bytesDownloaded: number;
  bytesTotal: number;
}

type CancellableOutcome =
  | { status: 'imported'; instance_id: string; warnings: string[] }
  | { status: 'updated'; instance_id: string; warnings: string[] }
  | { status: 'skipped'; reason: string }
  | { status: 'failed'; error: string; warnings: string[] }
  | { status: 'cancelled'; reason: string };

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatBytes(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)} KB`;
  return `${bytes} B`;
}

function formatTimestamp(iso: string | null): string {
  if (!iso) return 'Never';
  try {
    const numeric = /^\d+$/.test(iso) ? Number(iso) : null;
    const d = new Date(numeric == null ? iso : numeric < 1_000_000_000_000 ? numeric * 1000 : numeric);
    if (isNaN(d.getTime())) return 'Unknown';
    const diffMs = Date.now() - d.getTime();
    if (diffMs < 60_000) return 'Just now';
    if (diffMs < 3_600_000) return `${Math.floor(diffMs / 60_000)}m ago`;
    if (diffMs < 86_400_000) return `${Math.floor(diffMs / 3_600_000)}h ago`;
    if (diffMs < 604_800_000) return `${Math.floor(diffMs / 86_400_000)}d ago`;
    return d.toLocaleDateString();
  } catch {
    return 'Unknown';
  }
}

function isSelectable(status: CandidateStatus): boolean {
  if (status === 'ready' || status === 'needs_review') return true;
  return false;
}

function getUnsupportedReasons(status: CandidateStatus): string[] {
  if (typeof status === 'object' && status !== null && 'unsupported' in status) {
    return (status as { unsupported: { reasons: string[] } }).unsupported.reasons;
  }
  return [];
}

function getLauncherGroups(discovery: LauncherImportDiscovery): LauncherGroup[] {
  return [
    {
      kind: 'prism',
      label: 'Prism Launcher',
      icon: <Layers className="h-4 w-4" />,
      detection: discovery.prism,
      candidates: discovery.prism.candidates,
    },
    {
      kind: 'curse_forge',
      label: 'CurseForge',
      icon: <Flame className="h-4 w-4" />,
      detection: discovery.curseforge,
      candidates: discovery.curseforge.candidates,
    },
    {
      kind: 'modrinth',
      label: 'Modrinth App',
      icon: <Gem className="h-4 w-4" />,
      detection: discovery.modrinth,
      candidates: discovery.modrinth.candidates,
    },
  ];
}

function candidateCompositeKey(c: { launcher: string; launcher_installation_key: string; source_key: string }): string {
  return `${c.launcher}::${c.launcher_installation_key}::${c.source_key}`;
}

function mergeDiscovery(
  current: LauncherImportDiscovery,
  incoming: LauncherImportDiscovery,
): LauncherImportDiscovery {
  const mergeDisc = (a: LauncherDiscovery, b: LauncherDiscovery): LauncherDiscovery => {
    const launcher = b.launcher ?? a.launcher;
    const map = new Map<string, LauncherImportCandidate>();
    for (const c of a.candidates) map.set(candidateCompositeKey(c), c);
    for (const c of b.candidates) map.set(candidateCompositeKey(c), c);
    return {
      launcher: launcher ? {
        ...launcher,
        detection_warnings: Array.from(new Set([
          ...(a.launcher?.detection_warnings ?? []),
          ...(b.launcher?.detection_warnings ?? []),
        ])),
      } : null,
      candidates: Array.from(map.values()),
    };
  };
  return {
    prism: mergeDisc(current.prism, incoming.prism),
    curseforge: mergeDisc(current.curseforge, incoming.curseforge),
    modrinth: mergeDisc(current.modrinth, incoming.modrinth),
  };
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface LauncherImportWizardProps {
  open: boolean;
  onClose: () => void;
  onComplete: () => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function LauncherImportWizard({ open, onClose, onComplete }: LauncherImportWizardProps) {
  const [stage, setStage] = useState<Stage>('detect');
  const [discovery, setDiscovery] = useState<LauncherImportDiscovery | null>(null);
  const [detectionError, setDetectionError] = useState<string | null>(null);
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set());
  const [destinationNames, setDestinationNames] = useState<Record<string, string>>({});
  const [preserveSettings, setPreserveSettings] = useState(true);
  const [searchQuery, setSearchQuery] = useState('');
  const [customRootErrors, setCustomRootErrors] = useState<string[]>([]);
  const [plan, setPlan] = useState<LauncherImportPlan | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [outcomes, setOutcomes] = useState<CancellableOutcome[] | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [progress, setProgress] = useState<OperationProgressPayload | null>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const importBusyRef = useRef(false);
  const cancellationRequestedRef = useRef(false);
  const operationIdRef = useRef<string | null>(null);
  const eventGatePassedRef = useRef(false);
  const unlistenRef = useRef<(() => void) | null>(null);

  // Init & detect on first open
  useEffect(() => {
    if (!open) return;
    setStage('detect');
    setDiscovery(null);
    setDetectionError(null);
    setSelectedKeys(new Set());
    setDestinationNames({});
    setPreserveSettings(true);
    setSearchQuery('');
    setCustomRootErrors([]);
    setPlan(null);
    setPlanError(null);
    setImporting(false);
    setCancelling(false);
    setOutcomes(null);
    setImportError(null);
    setProgress(null);
    operationIdRef.current = null;
    eventGatePassedRef.current = false;
    cancellationRequestedRef.current = false;

    let cancelled = false;
    (async () => {
      try {
        const d = await discoverLauncherImports();
        if (!cancelled) {
          setDiscovery(d);
          setStage('select');
        }
      } catch (e) {
        if (!cancelled) setDetectionError(formatError(e));
      }
    })();
    return () => { cancelled = true; };
  }, [open]);

  // Focus heading on stage change
  useEffect(() => {
    const id = setTimeout(() => headingRef.current?.focus(), 50);
    return () => clearTimeout(id);
  }, [stage]);

  // Subscribe while the dialog is open so the listener is ready before the
  // backend can emit the first import event.
  useEffect(() => {
    if (!open) return;

    let active = true;
    (async () => {
      const unlisten = await listen<OperationProgressPayload>('operation-progress', (event) => {
        if (!active) return;
        if (!importBusyRef.current) return;
        const p = event.payload;

        if (!eventGatePassedRef.current) {
          const msg = (p.message || '').toLowerCase();
          if (!msg.includes('importing') && !msg.includes('launcher import')) return;
          eventGatePassedRef.current = true;
        }

        operationIdRef.current = p.operationId;
        setProgress(p);
      });
      if (active) {
        unlistenRef.current = unlisten;
      } else {
        unlisten();
      }
    })();

    return () => {
      active = false;
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, [open]);

  // Derived data
  const groups = useMemo(() => {
    if (!discovery) return [];
    return getLauncherGroups(discovery);
  }, [discovery]);

  const allCandidates = useMemo(() => groups.flatMap(g => g.candidates), [groups]);

  const candidateMap = useMemo(() => {
    const map = new Map<string, LauncherImportCandidate>();
    for (const c of allCandidates) map.set(candidateCompositeKey(c), c);
    return map;
  }, [allCandidates]);

  const filteredSourceKeys = useMemo(() => {
    if (!searchQuery) return null;
    const q = searchQuery.toLowerCase();
    return new Set(
      allCandidates
        .filter(c => c.display_name.toLowerCase().includes(q))
        .map(c => candidateCompositeKey(c)),
    );
  }, [allCandidates, searchQuery]);

  const hasSelection = selectedKeys.size > 0;

  // Callbacks
  const handleClose = useCallback(() => {
    if (importing || cancelling) {
      cancellationRequestedRef.current = true;
      const opId = operationIdRef.current;
      if (opId) { cancelOperation(opId).catch(() => {}); }
    }
    unlistenRef.current?.();
    unlistenRef.current = null;
    onClose();
  }, [importing, cancelling, onClose]);

  const handleAddLocation = useCallback(async () => {
    try {
      const path = await pickDirectory('Select launcher instance directory');
      if (!path) return;
      const r = await discoverLauncherImports(path);
      setDiscovery(prev => prev ? mergeDiscovery(prev, r) : r);
    } catch (e) {
      setCustomRootErrors(prev => [...prev, formatError(e)]);
    }
  }, []);

  const handleToggleCandidate = useCallback((compositeKey: string) => {
    setSelectedKeys(prev => {
      const next = new Set(prev);
      if (next.has(compositeKey)) next.delete(compositeKey);
      else next.add(compositeKey);
      return next;
    });
  }, []);

  const handleSelectAll = useCallback((candidates: LauncherImportCandidate[]) => {
    setSelectedKeys(prev => {
      const next = new Set(prev);
      for (const c of candidates) {
        if (isSelectable(c.status)) next.add(candidateCompositeKey(c));
      }
      return next;
    });
  }, []);

  const handleClearGroup = useCallback((candidates: LauncherImportCandidate[]) => {
    setSelectedKeys(prev => {
      const next = new Set(prev);
      for (const c of candidates) next.delete(candidateCompositeKey(c));
      return next;
    });
  }, []);

  const handleDestinationNameChange = useCallback((compositeKey: string, name: string) => {
    setDestinationNames(prev => ({ ...prev, [compositeKey]: name }));
  }, []);

  const buildSelections = useCallback((): ImportSelection[] => {
    const selections: ImportSelection[] = [];
    for (const key of selectedKeys) {
      const candidate = candidateMap.get(key);
      if (!candidate) continue;
      selections.push({
        source_key: candidate.source_key,
        launcher_kind: candidate.launcher,
        installation_key: candidate.launcher_installation_key,
        destination_name: destinationNames[key] ?? candidate.display_name,
        preserve_settings: preserveSettings,
      });
    }
    return selections;
  }, [selectedKeys, candidateMap, destinationNames, preserveSettings]);

  const handleReview = useCallback(async () => {
    setPlanError(null);
    setPlan(null);
    setStage('review');
    try {
      const selections = buildSelections();
      const p = await planLauncherImports(selections);
      setPlan(p);
    } catch (e) {
      setPlanError(formatError(e));
    }
  }, [buildSelections]);

  const handleBackToSelect = useCallback(() => {
    setPlan(null);
    setPlanError(null);
    setStage('select');
  }, []);

  const handleCancelImport = useCallback(async () => {
    const opId = operationIdRef.current;
    if (!opId || cancelling) return;
    setCancelling(true);
    cancellationRequestedRef.current = true;
    try {
      await cancelOperation(opId);
    } catch {
      // The backend may already have finished — the executeLauncherImports
      // promise settles shortly after; the result determines the outcome.
    }
  }, [cancelling]);

  const handleImport = useCallback(async () => {
    if (!plan || importBusyRef.current) return;
    importBusyRef.current = true;
    setImporting(true);
    setCancelling(false);
    setImportError(null);
    setProgress(null);
    operationIdRef.current = null;
    eventGatePassedRef.current = false;
    cancellationRequestedRef.current = false;
    setStage('importing');
    try {
      const r = await executeLauncherImports(plan);
      setOutcomes(r.outcomes);
    } catch (e) {
      if (cancellationRequestedRef.current) {
        setOutcomes([{ status: 'cancelled' as const, reason: 'Import was cancelled by the user.' }]);
      } else {
        setImportError(formatError(e));
      }
    } finally {
      setImporting(false);
      setCancelling(false);
      importBusyRef.current = false;
      setStage('results');
    }
  }, [plan]);

  const handleDone = useCallback(() => {
    unlistenRef.current?.();
    unlistenRef.current = null;
    onComplete();
    onClose();
  }, [onComplete, onClose]);

  const handleRetryDetection = useCallback(async () => {
    setDetectionError(null);
    setStage('detect');
    try {
      const d = await discoverLauncherImports();
      setDiscovery(d);
      setStage('select');
    } catch (e) {
      setDetectionError(formatError(e));
    }
  }, []);

  // ── Render: nothing when closed ─────────────────────
  if (!open) return null;

  // ── Render helpers ──────────────────────────────────

  const renderContentIndicators = (inventory: LauncherImportCandidate['inventory']) => {
    const items: { icon: React.ReactNode; label: string; active: boolean }[] = [
      { icon: <Package className="h-3 w-3" />, label: 'Mods', active: inventory.has_mods },
      { icon: <Image className="h-3 w-3" />, label: 'RP', active: inventory.has_resourcepacks },
      { icon: <Sun className="h-3 w-3" />, label: 'Shaders', active: inventory.has_shaderpacks },
      { icon: <Database className="h-3 w-3" />, label: 'Data', active: inventory.has_datapacks },
      { icon: <Save className="h-3 w-3" />, label: 'Saves', active: inventory.has_saves },
    ];
    return (
      <div className="flex flex-wrap gap-1.5">
        {items.map(item => item.active && (
          <span
            key={item.label}
            className="inline-flex items-center gap-0.5 rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground"
          >
            {item.icon}
            {item.label}
          </span>
        ))}
      </div>
    );
  };

  const renderLoaderLabel = (candidate: LauncherImportCandidate) => {
    const lt = candidate.loader_tuple;
    if (!lt) return <span className="text-xs text-muted-foreground">Unknown version</span>;
    return (
      <span className="text-xs text-muted-foreground">
        {lt.minecraft_version} &middot; {lt.loader} {lt.loader_version}
      </span>
    );
  };

  const renderItemStatus = (status: CandidateStatus) => {
    if (status === 'ready') {
      return <span className="text-xs text-green-600 dark:text-green-400 font-medium">Ready</span>;
    }
    if (status === 'needs_review') {
      return <span className="inline-flex items-center gap-1 text-xs text-amber-600 dark:text-amber-400 font-medium"><Info className="h-3 w-3" />Needs review</span>;
    }
    const reasons = getUnsupportedReasons(status);
    return (
      <div className="space-y-0.5">
        <span className="text-xs text-destructive font-medium">Unsupported</span>
        {reasons.map((r, i) => (
          <p key={i} className="text-[10px] text-muted-foreground leading-tight">{r}</p>
        ))}
      </div>
    );
  };

  // ── Detect stage ────────────────────────────────────
  const renderDetect = () => (
    <div className="flex flex-col items-center justify-center py-12 gap-3">
      {detectionError ? (
        <>
          <AlertTriangle className="h-8 w-8 text-destructive" />
          <p className="text-sm text-destructive text-center max-w-md">{detectionError}</p>
          <Button variant="outline" size="sm" onClick={handleRetryDetection}>Retry Detection</Button>
        </>
      ) : (
        <>
          <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          <p className="text-sm text-muted-foreground">Scanning for local launcher installations&hellip;</p>
        </>
      )}
    </div>
  );

  // ── Select stage ────────────────────────────────────
  const renderSelect = () => {
    const ck = candidateCompositeKey;

    return (
      <div className="space-y-4">
        {/* Toolbar */}
        <div className="flex flex-col sm:flex-row gap-2">
          <div className="relative flex-1">
            <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground pointer-events-none" />
            <Input
              value={searchQuery}
              onChange={e => setSearchQuery(e.target.value)}
              placeholder="Search instances&hellip;"
              className="pl-8 h-9 text-sm"
              aria-label="Search instances"
            />
          </div>
          <Button variant="outline" size="sm" onClick={handleAddLocation} className="shrink-0 gap-1.5">
            <FolderPlus className="h-4 w-4" />
            Add location
          </Button>
        </div>

        {/* Custom root errors */}
        {customRootErrors.length > 0 && (
          <div className="rounded-lg bg-destructive/10 p-2 text-xs text-destructive space-y-0.5">
            {customRootErrors.map((err, i) => <p key={i}>{err}</p>)}
          </div>
        )}

        {/* Preserve settings toggle */}
        <div className="flex items-center gap-2">
          <Switch
            id="preserve-settings"
            checked={preserveSettings}
            onCheckedChange={setPreserveSettings}
          />
          <label htmlFor="preserve-settings" className="text-xs text-muted-foreground cursor-pointer">
            Preserve compatible settings (memory, Java path, JVM args)
          </label>
        </div>

        {/* Summary row */}
        {hasSelection && (
          <p className="text-xs text-muted-foreground">
            {selectedKeys.size} instance{selectedKeys.size !== 1 ? 's' : ''} selected
            &nbsp;&middot; {formatBytes(totalSelectedBytes())}
            &nbsp;&middot; {totalSelectedFiles()} file{totalSelectedFiles() !== 1 ? 's' : ''}
          </p>
        )}

        {/* Launcher groups */}
        {groups.map(group => {
          const visible = group.candidates.filter(c => !filteredSourceKeys || filteredSourceKeys.has(ck(c)));
          const selectableInGroup = group.candidates.filter(c => isSelectable(c.status));
          if (visible.length === 0) return null;

          return (
            <div key={group.kind} className="rounded-lg border border-border">
              {/* Group header */}
              <div className="flex items-center justify-between gap-2 px-3 py-2 border-b border-border bg-muted/30">
                <div className="flex items-center gap-2 min-w-0">
                  <span className="text-muted-foreground shrink-0">{group.icon}</span>
                  <span className="text-sm font-semibold truncate">{group.label}</span>
                  {group.detection.launcher && (
                    <span className="text-[10px] text-muted-foreground shrink-0">
                      ({group.detection.launcher.instance_count} instance{group.detection.launcher.instance_count !== 1 ? 's' : ''})
                    </span>
                  )}
                  {group.detection.launcher && group.detection.launcher.detection_warnings.length > 0 && (
                    <span className="inline-flex items-center gap-1 text-[10px] text-amber-600 dark:text-amber-400" title={group.detection.launcher.detection_warnings.join('\n')}>
                      <AlertTriangle className="h-3 w-3" />
                    </span>
                  )}
                </div>
                {selectableInGroup.length > 0 && (
                  <div className="flex items-center gap-1 shrink-0">
                    <button
                      type="button"
                      onClick={() => handleSelectAll(selectableInGroup)}
                      className="text-[11px] font-medium text-primary hover:underline px-1.5 py-0.5 rounded hover:bg-accent"
                    >
                      Select all
                    </button>
                    <span className="text-[10px] text-muted-foreground">&middot;</span>
                    <button
                      type="button"
                      onClick={() => handleClearGroup(selectableInGroup)}
                      className="text-[11px] font-medium text-muted-foreground hover:text-foreground hover:underline px-1.5 py-0.5 rounded hover:bg-accent"
                    >
                      Clear
                    </button>
                  </div>
                )}
              </div>

              {/* Candidates */}
              <div className="divide-y divide-border">
                {visible.map(candidate => {
                  const key = ck(candidate);
                  const selectable = isSelectable(candidate.status);
                  const checked = selectedKeys.has(key);
                  return (
                    <div
                      key={key}
                      className={cn(
                        'flex items-start gap-3 p-3 transition-colors',
                        !selectable && 'opacity-60',
                      )}
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        disabled={!selectable}
                        onChange={() => handleToggleCandidate(key)}
                        className="mt-1 h-4 w-4 rounded border-gray-300 text-primary focus:ring-primary disabled:opacity-40"
                        aria-label={`Select ${candidate.display_name}`}
                      />
                      <div className="flex-1 min-w-0 space-y-1.5">
                        {/* Top row: name + status */}
                        <div className="flex items-center gap-2 flex-wrap">
                          <span className="text-sm font-medium truncate">{candidate.display_name}</span>
                          {renderItemStatus(candidate.status)}
                        </div>

                        {/* MC + loader */}
                        {renderLoaderLabel(candidate)}

                        {/* Content indicators */}
                        {renderContentIndicators(candidate.inventory)}

                        {/* Size + files + last played */}
                        <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground">
                          <span>{formatBytes(candidate.inventory.total_bytes)}</span>
                          <span>{candidate.inventory.total_files} file{candidate.inventory.total_files !== 1 ? 's' : ''}</span>
                          <span>Last played: {formatTimestamp(candidate.last_played)}</span>
                        </div>

                        {/* Source label */}
                        <span className="inline-flex items-center gap-1 text-[10px] text-muted-foreground">
                          {group.icon}
                          {group.label}
                          {candidate.payload_root && (
                            <span className="truncate max-w-[200px]" title={candidate.payload_root}>
                              &middot; {candidate.payload_root}
                            </span>
                          )}
                        </span>

                        {/* Warnings */}
                        {candidate.warnings.length > 0 && (
                          <div className="space-y-0.5">
                            {candidate.warnings.map((w, i) => (
                              <p key={i} className="flex items-start gap-1 text-[11px] text-amber-600 dark:text-amber-400">
                                <AlertTriangle className="h-3 w-3 mt-0.5 shrink-0" />
                                {w}
                              </p>
                            ))}
                          </div>
                        )}

                        {/* Editable destination name (only when selected) */}
                        {checked && (
                          <div className="flex items-center gap-2 pt-1">
                            <span className="text-[11px] text-muted-foreground shrink-0">Import as:</span>
                            <Input
                              value={destinationNames[key] ?? candidate.display_name}
                              onChange={e => handleDestinationNameChange(key, e.target.value)}
                              className="h-7 text-xs max-w-xs"
                              aria-label={`Destination name for ${candidate.display_name}`}
                            />
                          </div>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}

        {/* No candidates at all */}
        {allCandidates.length === 0 && !searchQuery && (
          <div className="flex flex-col items-center justify-center py-12 gap-2">
            <Info className="h-6 w-6 text-muted-foreground" />
            <p className="text-sm text-muted-foreground">No launcher installations detected.</p>
            <p className="text-xs text-muted-foreground">Try adding a custom launcher location above.</p>
          </div>
        )}

        {/* Search no results */}
        {allCandidates.length > 0 && searchQuery && filteredSourceKeys?.size === 0 && (
          <div className="flex flex-col items-center justify-center py-8 gap-2">
            <Search className="h-5 w-5 text-muted-foreground" />
            <p className="text-sm text-muted-foreground">No instances match &ldquo;{searchQuery}&rdquo;</p>
          </div>
        )}

        {/* Selected candidates summary + Review button */}
        {hasSelection && (
          <div className="sticky bottom-0 bg-background pt-3 pb-1 border-t border-border -mx-6 px-6">
            <div className="flex items-center justify-between">
              <p className="text-xs text-muted-foreground">
                {selectedKeys.size} instance{selectedKeys.size !== 1 ? 's' : ''} selected
                &nbsp;&middot; {formatBytes(totalSelectedBytes())}
              </p>
              <Button size="sm" onClick={handleReview}>
                Review ({selectedKeys.size})
              </Button>
            </div>
          </div>
        )}
      </div>
    );
  };

  // ── Review stage ────────────────────────────────────
  const renderReview = () => (
    <div className="space-y-4">
      {/* Loading plan */}
      {!plan && !planError && (
        <div className="flex flex-col items-center justify-center py-8 gap-3">
          <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          <p className="text-sm text-muted-foreground">Building import plan&hellip;</p>
        </div>
      )}

      {/* Plan error */}
      {planError && (
        <div className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">{planError}</div>
      )}

      {/* Plan content */}
      {plan && (
        <>
          {/* Batch blockers */}
          {plan.batch_blockers.length > 0 && (
            <div className="rounded-lg bg-destructive/10 p-3 space-y-1">
              <p className="text-sm font-semibold text-destructive">Import Blocked</p>
              {plan.batch_blockers.map((b, i) => (
                <p key={i} className="text-xs text-destructive/90">{b}</p>
              ))}
            </div>
          )}

          {/* Plan items */}
          {plan.items.length === 0 && (
            <div className="rounded-lg border border-dashed border-border p-6 text-center text-sm text-muted-foreground">
              No instances are available in this import plan. Return to selection and review any
              unsupported-instance messages.
            </div>
          )}
          <div className="space-y-2">
            {plan.items.map(item => (
              <div
                key={item.fingerprint}
                className="rounded-lg border border-border p-3 space-y-2"
              >
                {/* Action badge + name */}
                <div className="flex items-center gap-2 flex-wrap">
                  <span className={cn(
                    'inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wider',
                    item.action === 'new'
                      ? 'bg-green-500/10 text-green-700 dark:text-green-400'
                      : 'bg-blue-500/10 text-blue-700 dark:text-blue-400',
                  )}>
                    {item.action}
                  </span>
                  <span className="text-sm font-medium">{item.destination_name}</span>
                </div>

                {/* Loader tuple */}
                {item.loader_tuple && (
                  <p className="text-xs text-muted-foreground">
                    {item.loader_tuple.minecraft_version} &middot; {item.loader_tuple.loader} {item.loader_tuple.loader_version}
                  </p>
                )}

                {/* Settings preview */}
                {item.preserve_settings && (
                  <div className="text-[11px] text-muted-foreground space-y-0.5">
                    {item.sanitized_settings.memory_mb != null && (
                      <p>Memory: {item.sanitized_settings.memory_mb} MB</p>
                    )}
                    {item.sanitized_settings.java_path && (
                      <p className="truncate" title={item.sanitized_settings.java_path}>Java: {item.sanitized_settings.java_path}</p>
                    )}
                    {item.sanitized_settings.jvm_args.length > 0 && (
                      <p className="truncate" title={item.sanitized_settings.jvm_args.join(' ')}>JVM args: {item.sanitized_settings.jvm_args.join(' ')}</p>
                    )}
                  </div>
                )}

                {/* Blockers */}
                {item.blockers.length > 0 && (
                  <div className="space-y-0.5">
                    {item.blockers.map((b, i) => (
                      <p key={i} className="flex items-start gap-1 text-[11px] text-destructive">
                        <XCircle className="h-3 w-3 mt-0.5 shrink-0" />
                        {b}
                      </p>
                    ))}
                  </div>
                )}

                {/* Warnings */}
                {item.warnings.length > 0 && (
                  <div className="space-y-0.5">
                    {item.warnings.map((w, i) => (
                      <p key={i} className="flex items-start gap-1 text-[11px] text-amber-600 dark:text-amber-400">
                        <AlertTriangle className="h-3 w-3 mt-0.5 shrink-0" />
                        {w}
                      </p>
                    ))}
                  </div>
                )}

                {/* Size */}
                <p className="text-[11px] text-muted-foreground">
                  {formatBytes(item.total_bytes)} &middot; {item.total_files} file{item.total_files !== 1 ? 's' : ''}
                </p>
              </div>
            ))}
          </div>

          {/* Summary footer */}
          <div className="rounded-lg bg-muted/50 p-3 space-y-1">
            <div className="flex items-center justify-between text-xs text-muted-foreground">
              <span>Total items: {plan.items.length}</span>
              <span>Peak storage: {formatBytes(plan.peak_bytes)}</span>
              <span>Total files: {plan.total_files}</span>
            </div>
          </div>

          {/* Actions */}
          <div className="flex justify-end gap-2 pt-2 border-t border-border">
            <Button variant="outline" size="sm" onClick={handleBackToSelect}>Back</Button>
            <Button
              size="sm"
              onClick={handleImport}
              disabled={plan.batch_blockers.length > 0 || plan.items.length === 0}
            >
              Import {plan.items.length} Instance{plan.items.length !== 1 ? 's' : ''}
            </Button>
          </div>
        </>
      )}
    </div>
  );

  // ── Importing stage ─────────────────────────────────
  const renderImporting = () => {
    const p = progress;
    const pct = p ? Math.min(p.progress, 100) : 0;
    const showProgress = p && (p.bytesTotal > 0 || p.progress > 0);
    const phaseLabel = p?.phase ? p.phase.charAt(0).toUpperCase() + p.phase.slice(1) : 'Working';

    return (
      <div className="flex flex-col items-center justify-center py-8 gap-4">
        {!cancelling ? (
          <Loader2 className="h-8 w-8 animate-spin text-primary" />
        ) : (
          <Ban className="h-8 w-8 text-muted-foreground" />
        )}

        <div className="text-center space-y-1">
          <p className="text-sm font-medium">
            {cancelling ? 'Cancelling…' : phaseLabel}
          </p>
          {p?.message && (
            <p className="text-xs text-muted-foreground max-w-sm">{p.message}</p>
          )}
          {p?.subLabel && (
            <p className="text-xs text-muted-foreground/70">{p.subLabel}</p>
          )}
        </div>

        {showProgress && !cancelling && (
          <div className="w-full max-w-sm space-y-1.5">
            <div className="h-2 rounded-full bg-muted overflow-hidden">
              <div
                className="h-full bg-primary transition-all duration-300 rounded-full"
                style={{ width: `${pct}%` }}
              />
            </div>
            <div className="flex justify-between text-[10px] text-muted-foreground">
              <span>{pct}%</span>
              {p.bytesTotal > 0 && (
                <span>{formatBytes(p.bytesDownloaded)} / {formatBytes(p.bytesTotal)}</span>
              )}
            </div>
          </div>
        )}

        <Button
          variant="outline"
          size="sm"
          onClick={handleCancelImport}
          disabled={cancelling}
          className="gap-1.5"
        >
          {cancelling ? 'Cancelling…' : 'Cancel'}
        </Button>
      </div>
    );
  };

  // ── Results stage ───────────────────────────────────
  const renderResults = () => (
    <div className="space-y-4">
      {/* Import error (non-cancellation) */}
      {importError && (
        <div className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">{importError}</div>
      )}

      {/* Outcomes array */}
      {outcomes && outcomes.length > 0 && (
        <div className="space-y-2">
          {outcomes.map((outcome, i) => {
            const isCancelled = outcome.status === 'cancelled';
            return (
              <div
                key={i}
                className={cn(
                  'rounded-lg border p-3 space-y-1',
                  outcome.status === 'imported' && 'border-green-500/30 bg-green-500/5',
                  outcome.status === 'updated' && 'border-blue-500/30 bg-blue-500/5',
                  outcome.status === 'skipped' && 'border-border bg-muted/30',
                  outcome.status === 'failed' && 'border-destructive/30 bg-destructive/5',
                  isCancelled && 'border-muted-foreground/30 bg-muted/20',
                )}
              >
                <div className="flex items-center gap-2">
                  {outcome.status === 'imported' && <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />}
                  {outcome.status === 'updated' && <CheckCircle2 className="h-4 w-4 text-blue-600 dark:text-blue-400 shrink-0" />}
                  {outcome.status === 'skipped' && <Info className="h-4 w-4 text-muted-foreground shrink-0" />}
                  {outcome.status === 'failed' && <XCircle className="h-4 w-4 text-destructive shrink-0" />}
                  {isCancelled && <Ban className="h-4 w-4 text-muted-foreground shrink-0" />}
                  <span className="text-sm font-medium">
                    {outcome.status === 'imported' && 'Imported'}
                    {outcome.status === 'updated' && 'Updated'}
                    {outcome.status === 'skipped' && 'Skipped'}
                    {outcome.status === 'failed' && 'Failed'}
                    {isCancelled && 'Cancelled'}
                  </span>
                  {outcome.status === 'imported' && 'instance_id' in outcome && (
                    <span className="text-xs text-muted-foreground truncate">{outcome.instance_id}</span>
                  )}
                  {outcome.status === 'updated' && 'instance_id' in outcome && (
                    <span className="text-xs text-muted-foreground truncate">{outcome.instance_id}</span>
                  )}
                </div>

                {outcome.status === 'skipped' && 'reason' in outcome && outcome.reason && (
                  <p className="text-xs text-muted-foreground">{outcome.reason}</p>
                )}
                {outcome.status === 'failed' && 'error' in outcome && outcome.error && (
                  <p className="text-xs text-destructive">{outcome.error}</p>
                )}
                {isCancelled && 'reason' in outcome && outcome.reason && (
                  <p className="text-xs text-muted-foreground">{outcome.reason}</p>
                )}

                {'warnings' in outcome && outcome.warnings && outcome.warnings.length > 0 && (
                  <div className="space-y-0.5 pt-1">
                    {outcome.warnings.map((w, j) => (
                      <p key={j} className="flex items-start gap-1 text-[11px] text-amber-600 dark:text-amber-400">
                        <AlertTriangle className="h-3 w-3 mt-0.5 shrink-0" />
                        {w}
                      </p>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* No outcomes at all */}
      {(!outcomes || outcomes.length === 0) && !importError && (
        <div className="flex flex-col items-center justify-center py-8 gap-2">
          <Info className="h-5 w-5 text-muted-foreground" />
          <p className="text-sm text-muted-foreground">No instances were imported.</p>
        </div>
      )}

      {/* Done button */}
      <div className="flex justify-end pt-2 border-t border-border">
        <Button size="sm" onClick={handleDone}>Done</Button>
      </div>
    </div>
  );

  // ── Total selected helpers ──────────────────────────
  function totalSelectedBytes(): number {
    let total = 0;
    for (const key of selectedKeys) {
      const c = candidateMap.get(key);
      if (c) total += c.inventory.total_bytes;
    }
    return total;
  }

  function totalSelectedFiles(): number {
    let total = 0;
    for (const key of selectedKeys) {
      const c = candidateMap.get(key);
      if (c) total += c.inventory.total_files;
    }
    return total;
  }

  // ── Main render ─────────────────────────────────────
  const isLocked = importing || cancelling;

  return (
    <Dialog open onOpenChange={(o) => { if (!o) handleClose(); }}>
      <DialogContent
        className="max-w-4xl max-h-[85vh] flex flex-col p-0 gap-0"
        onEscapeKeyDown={isLocked ? (e) => e.preventDefault() : undefined}
        onInteractOutside={isLocked ? (e) => e.preventDefault() : undefined}
      >
        {/* Header bar */}
        <div className="flex items-center gap-2 px-6 pt-6 pb-3 border-b border-border shrink-0">
          {stage === 'review' && !isLocked && (
            <button
              type="button"
              onClick={handleBackToSelect}
              className="rounded-md p-1 -ml-1 hover:bg-accent transition-colors"
              aria-label="Back to selection"
            >
              <ArrowLeft className="h-4 w-4" />
            </button>
          )}
          <DialogTitle ref={headingRef} tabIndex={-1} className="text-base font-semibold outline-none">
            {stage === 'detect' && 'Detecting Launchers'}
            {stage === 'select' && 'Import from Other Launchers'}
            {stage === 'review' && 'Review Import Plan'}
            {stage === 'importing' && 'Importing Instances'}
            {stage === 'results' && 'Import Results'}
          </DialogTitle>
        </div>

        {/* Scrollable body */}
        <div className="flex-1 min-h-0 overflow-y-auto px-6 py-4">
          {stage === 'detect' && renderDetect()}
          {stage === 'select' && renderSelect()}
          {stage === 'review' && renderReview()}
          {stage === 'importing' && renderImporting()}
          {stage === 'results' && renderResults()}
        </div>
      </DialogContent>
    </Dialog>
  );
}
