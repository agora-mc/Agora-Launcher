import { useCallback, useEffect, useRef, useState } from 'react';
import { useRegistryState } from '../lib/useRegistryState';
import { RegistryStatusView } from '../components/registry-status-view';
import {
  checkInstanceCrash,
  detectDrift,
  getLkgMarker,
  getSetting,
  listInstances,
  listSnapshots,
  restoreSnapshot,
  forYouItems,
  setSetting,
  type InstanceRow,
  type RegistryItem,
} from '../lib/tauri';
import type { Tab } from '../lib/useDestination';
import type { ProcessState } from '../lib/useProcessController';
import { ArrowRight, BookOpen, GraduationCap } from 'lucide-react';

// ---------------------------------------------------------------------------
// D1: Action-oriented Home
// 4-zone layout: Alerts → Hero → Maintenance → Discovery
// ---------------------------------------------------------------------------

export function Home({
  onNavigateTab,
  onOpenInstance,
  onOpenMod,
  onLaunch,
  processState,
  onKillProcess,
}: {
  onNavigateTab: (tab: Tab) => void;
  onOpenInstance: (instanceId: string) => void;
  onOpenMod: (itemId: string) => void;
  onLaunch: (instanceId: string, directLaunch: boolean) => Promise<boolean>;
  processState: ProcessState;
  onKillProcess: () => Promise<void>;
}) {
  const { state, status, error, hasCachedDb, actions } = useRegistryState();

  const [instances, setInstances] = useState<InstanceRow[]>([]);
  const [instancesLoading, setInstancesLoading] = useState(true);
  const [lastCrash, setLastCrash] = useState<{ instanceId: string; name: string; filename?: string } | null>(null);
  const [knownGood, setKnownGood] = useState<{
    instanceId: string;
    instanceName: string;
    id: string;
    label: string;
    promotedAt: string | null;
    added: number;
    removed: number;
    disabled: number;
    updated: number;
  }[]>([]);
  const [knownGoodChecked, setKnownGoodChecked] = useState(false);
  const [recommendations, setRecommendations] = useState<RegistryItem[]>([]);
  const [recommendationsLoading, setRecommendationsLoading] = useState(false);
  const [restoringSnapshotId, setRestoringSnapshotId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const loadInFlightRef = useRef<Promise<void> | null>(null);
  const backgroundLoadInFlightRef = useRef<Promise<void> | null>(null);

  // Drift checks hash the tracked instance files and must not delay the first
  // usable Home render. Keep this work in the background and share it across
  // Strict Mode's development-only effect replay.
  const loadBackgroundData = useCallback(async (all: InstanceRow[]) => {
    if (backgroundLoadInFlightRef.current) return backgroundLoadInFlightRef.current;

    const task = (async () => {
      const launched = all.filter((i) => i.last_launched_at).sort(
        (a, b) => new Date(b.last_launched_at!).getTime() - new Date(a.last_launched_at!).getTime(),
      );

      const crashPromise = launched.length > 0
        ? checkInstanceCrash(launched[0].instance_id).catch(() => null)
        : Promise.resolve(null);
      const lkgPromise = (async () => {
        // Resolve exact promoted LKG pointers and current drift, never arbitrary snapshots.
        const lkgResults: typeof knownGood = [];
        for (const inst of all.slice(0, 5)) {
          try {
            const marker = await getLkgMarker(inst.instance_id);
            const snapshotId = typeof marker?.currentLkgSnapshotId === 'string'
              ? marker.currentLkgSnapshotId
              : null;
            if (!snapshotId) continue;
            const snapList = await listSnapshots(inst.instance_id);
            const snapshot = snapList.find((candidate) => candidate.id === snapshotId);
            const diff = await detectDrift(inst.instance_id, snapshotId);
            const entries = (key: 'added' | 'removed' | 'modified') => diff[key];
            const addedEntries = entries('added');
            const removedEntries = entries('removed');
            const removedPaths = new Set(removedEntries.map((entry) => String(entry.path ?? '')));
            const disabled = addedEntries.filter((entry) => {
              const path = String(entry.path ?? '');
              return path.endsWith('.disabled') && removedPaths.has(path.slice(0, -'.disabled'.length));
            }).length;
            lkgResults.push({
              instanceId: inst.instance_id,
              instanceName: inst.name,
              id: snapshotId,
              label: snapshot?.label ?? 'Last known good',
              promotedAt: typeof marker?.lastPromotedAt === 'string' ? marker.lastPromotedAt : null,
              added: addedEntries.length - disabled,
              removed: removedEntries.length - disabled,
              disabled,
              updated: entries('modified').length,
            });
          } catch { /* skip */ }
        }
        return lkgResults;
      })();

      const [crash, lkgResults] = await Promise.all([crashPromise, lkgPromise]);
      if (crash) {
        setLastCrash({ instanceId: launched[0].instance_id, name: launched[0].name, filename: crash.filename ?? undefined });
      } else {
        setLastCrash(null);
      }
      setKnownGood(lkgResults);
      setKnownGoodChecked(true);
    })();

    backgroundLoadInFlightRef.current = task;
    task.then(
      () => {
        if (backgroundLoadInFlightRef.current === task) backgroundLoadInFlightRef.current = null;
      },
      () => {
        if (backgroundLoadInFlightRef.current === task) backgroundLoadInFlightRef.current = null;
        setKnownGoodChecked(true);
      },
    );
    return task;
  }, []);

  // Load the lightweight instance list on mount. Registry status changes only
  // affect recommendations, not the instance/drift data already loaded here.
  const loadData = useCallback(async () => {
    if (loadInFlightRef.current) return loadInFlightRef.current;

    const task = (async () => {
      setInstancesLoading(true);
      setKnownGoodChecked(false);
      try {
        const all = await listInstances();
        setInstances(all);
        setInstancesLoading(false);
        void loadBackgroundData(all);
      } catch {
        setInstancesLoading(false);
        setKnownGoodChecked(true);
      }
    })();

    loadInFlightRef.current = task;
    task.then(
      () => {
        if (loadInFlightRef.current === task) loadInFlightRef.current = null;
      },
      () => {
        if (loadInFlightRef.current === task) loadInFlightRef.current = null;
      },
    );
    return task;
  }, [loadBackgroundData]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Recommendations are independent of crash/LKG inspection and may become
  // available after the registry status finishes loading.
  const sortedByLaunched = [...instances].sort(
    (a, b) => new Date(b.last_launched_at ?? 0).getTime() - new Date(a.last_launched_at ?? 0).getTime(),
  );
  const lastLaunched = sortedByLaunched.find((instance) => instance.last_launched_at) ?? null;
  const heroInstance = lastLaunched ?? sortedByLaunched[0] ?? null;

  useEffect(() => {
    if (!hasCachedDb || !lastLaunched) {
      setRecommendations([]);
      setRecommendationsLoading(false);
      return;
    }

    let cancelled = false;
    setRecommendationsLoading(true);
    (async () => {
      try {
        const modrinthEnabled = (await getSetting('modrinth_enabled')) === true;
        const result = await forYouItems(
          modrinthEnabled,
          lastLaunched.minecraft_version,
          lastLaunched.loader,
          3,
        );
        if (!cancelled) setRecommendations(result);
      } catch {
        if (!cancelled) setRecommendations([]);
      } finally {
        if (!cancelled) setRecommendationsLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [hasCachedDb, lastLaunched?.instance_id, lastLaunched?.minecraft_version, lastLaunched?.loader]);

  // Track last home visit for change detection.
  useEffect(() => {
    getSetting('last_home_visit').catch(() => {});
    return () => {
      setSetting('last_home_visit', new Date().toISOString()).catch(() => {});
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Group cards by zone.
  const crashKnownGood = lastCrash
    ? knownGood.find((entry) => entry.instanceId === lastCrash.instanceId) ?? null
    : null;

  const handleContinuePlaying = useCallback(async () => {
    if (!heroInstance) return;
    setActionError(null);
    try {
      const launchMode = await getSetting('launch_mode');
      await onLaunch(heroInstance.instance_id, launchMode === 'direct');
    } catch (error) {
      setActionError(error instanceof Error ? error.message : String(error));
    }
  }, [heroInstance, onLaunch]);

  const handleRestoreSnapshot = useCallback(async (snapshot: {
    instanceId: string;
    instanceName: string;
    id: string;
    label: string;
  }) => {
    const confirmed = window.confirm(
      `Restore "${snapshot.instanceName}" to snapshot "${snapshot.label || snapshot.id}"? Agora will create an undo snapshot first.`,
    );
    if (!confirmed) return;
    setActionError(null);
    setRestoringSnapshotId(snapshot.id);
    try {
      await restoreSnapshot(snapshot.instanceId, snapshot.id);
      await loadData();
    } catch (error) {
      setActionError(error instanceof Error ? error.message : String(error));
    } finally {
      setRestoringSnapshotId(null);
    }
  }, [loadData]);

  return (
    <div className="space-y-6">
      {/* Header */}
      <section className="agora-hero compact">
        <h2 className="text-2xl font-bold mb-2">Home</h2>
        <p className="text-muted-foreground">Your modding dashboard.</p>
      </section>

      {/* Zone A: Alerts — compact warnings */}
      {lastCrash && (
        <CrashAlert
          instanceName={lastCrash.name}
          crashFilename={lastCrash.filename}
          canRestore={Boolean(crashKnownGood)}
          onRestore={() => {
            if (crashKnownGood) {
              void handleRestoreSnapshot(crashKnownGood);
            } else {
              onOpenInstance(lastCrash.instanceId);
            }
          }}
        />
      )}

      {/* Zone A: Alerts — registry status with manual update check */}
      <RegistryStatusView
        variant="banner"
        state={state}
        status={status}
        error={error}
        actions={actions}
      />

      {/* Zone B: Hero — Continue Playing */}
      <ContinuePlayingCard
        instance={heroInstance}
        loading={instancesLoading}
        processState={processState}
        onLaunch={() => {
          if (heroInstance) {
            void handleContinuePlaying();
          }
        }}
        onKill={() => { void onKillProcess(); }}
        onBrowsePacks={() => onNavigateTab('browse')}
      />

      <GuideCard onOpenGuide={() => onNavigateTab('guide')} />

      {knownGood.length > 0 && (
        <KnownGoodCard
          snapshots={knownGood}
          restoringSnapshotId={restoringSnapshotId}
          onRestore={(snapshot) => void handleRestoreSnapshot(snapshot)}
        />
      )}
      {knownGoodChecked && instances.length > 0 && knownGood.length === 0 && (
        <div className="rounded-xl border border-dashed border-border bg-card p-4">
          <h4 className="text-sm font-semibold">No last-known-good state yet</h4>
          <p className="mt-1 text-xs text-muted-foreground">
            Play an instance successfully for at least 60 seconds. Agora will then promote its exact pre-launch snapshot for one-click recovery.
          </p>
        </div>
      )}

      {actionError && (
        <div role="alert" className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">
          {actionError}
        </div>
      )}

      {/* Zone D: Discovery — always present */}
      <RecommendationsCard
        hasInstances={instances.length > 0}
        hasCachedDb={hasCachedDb}
        loading={recommendationsLoading}
        activeInstance={lastLaunched}
        recommendations={recommendations}
        onOpenMod={onOpenMod}
        onBrowseMore={() => onNavigateTab('browse')}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Card components
// ---------------------------------------------------------------------------

function CrashAlert({ instanceName, crashFilename, canRestore, onRestore }: {
  instanceName: string;
  crashFilename?: string;
  canRestore: boolean;
  onRestore: () => void;
}) {
  return (
    <div className="rounded-lg border border-destructive bg-destructive/10 p-3 flex items-center justify-between gap-3">
      <div className="text-xs text-destructive flex-1">
        <span className="font-semibold">{instanceName}</span> did not exit cleanly.
        {crashFilename && <span className="text-muted-foreground ml-1">({crashFilename})</span>}
      </div>
      <button onClick={onRestore} className="rounded-lg bg-destructive px-3 py-1.5 text-xs font-medium text-destructive-foreground hover:bg-destructive/90">
        {canRestore ? 'View & restore' : 'View instance'}
      </button>
    </div>
  );
}

function ContinuePlayingCard({ instance, loading, processState, onLaunch, onKill, onBrowsePacks }: {
  instance: InstanceRow | null;
  loading: boolean;
  processState: ProcessState;
  onLaunch: () => void;
  onKill: () => void;
  onBrowsePacks: () => void;
}) {
  if (loading) {
    return (
      <div className="rounded-xl border border-border bg-card p-6 space-y-2">
        <div className="h-5 w-32 bg-muted animate-pulse rounded" />
        <div className="h-4 w-48 bg-muted animate-pulse rounded" />
      </div>
    );
  }

  if (!instance) {
    return (
      <div className="rounded-xl border border-border bg-card p-6">
        <h3 className="text-lg font-semibold mb-2">Welcome to Agora</h3>
        <p className="text-sm text-muted-foreground mb-4">
          No instances yet. Create one from a mod pack to start playing.
        </p>
        <button onClick={onBrowsePacks} className="rounded-lg bg-primary px-5 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90">
          Browse mod packs
        </button>
      </div>
    );
  }

  const timeAgo = instance.last_launched_at
    ? timeSince(new Date(instance.last_launched_at))
    : 'Not launched yet';
  const isCurrent = processState.instanceId === instance.instance_id;
  const isLaunching = isCurrent && processState.phase === 'launching';
  const isStopping = isCurrent && processState.phase === 'stopping';
  const isRunning = isCurrent && processState.phase === 'running';
  const isDelegated = isCurrent && processState.phase === 'delegated';
  const anotherProcessActive = processState.instanceId !== null
    && ['launching', 'running', 'stopping', 'delegated'].includes(processState.phase)
    && !isCurrent;
  const buttonDisabled = isLaunching || isStopping || isDelegated || anotherProcessActive;

  return (
    <div className="rounded-xl border border-border bg-card p-6">
      <h3 className="text-lg font-semibold mb-1">{instance.name}</h3>
      <p className="text-xs text-muted-foreground mb-1">
        {instance.loader} {instance.loader_version} · MC {instance.minecraft_version}
      </p>
      <p className="text-xs text-muted-foreground mb-4">{timeAgo}</p>
      <div className="flex gap-2">
        {isRunning ? (
          <button
            onClick={onKill}
            className="rounded-lg bg-destructive px-5 py-2 text-sm font-medium text-destructive-foreground hover:bg-destructive/90"
          >
            Kill
          </button>
        ) : (
          <button
            onClick={onLaunch}
            disabled={buttonDisabled}
            className="rounded-lg bg-primary px-5 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {isLaunching ? 'Starting…' : isStopping ? 'Stopping…' : isDelegated ? 'Running via Mojang' : anotherProcessActive ? 'Game already running' : 'Continue Playing'}
          </button>
        )}
      </div>
    </div>
  );
}

function GuideCard({ onOpenGuide }: { onOpenGuide: () => void }) {
  return (
    <div className="overflow-hidden rounded-xl border border-primary/20 bg-card">
      <div className="grid gap-4 p-5 sm:grid-cols-[auto_minmax(0,1fr)_auto] sm:items-center">
        <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-primary/10 text-primary">
          <BookOpen className="h-5 w-5" aria-hidden="true" />
        </div>
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="font-semibold">Learn Agora at your level</h3>
            <span className="inline-flex items-center gap-1 rounded-full bg-secondary px-2 py-0.5 text-[11px] font-semibold text-secondary-foreground">
              <GraduationCap className="h-3 w-3" aria-hidden="true" />
              36 guide pages
            </span>
          </div>
          <p className="mt-1 text-sm leading-5 text-muted-foreground">
            Follow beginner walkthroughs or open the advanced companion for every topic, from your first mod to JVM tuning and recovery.
          </p>
        </div>
        <button
          onClick={onOpenGuide}
          className="inline-flex items-center justify-center gap-2 rounded-lg bg-primary px-4 py-2 text-sm font-semibold text-primary-foreground hover:bg-primary/90"
        >
          Open Help & Guide
          <ArrowRight className="h-4 w-4" aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}

function KnownGoodCard({
  snapshots,
  restoringSnapshotId,
  onRestore,
}: {
  snapshots: {
    instanceId: string;
    instanceName: string;
    id: string;
    label: string;
    promotedAt: string | null;
    added: number;
    removed: number;
    disabled: number;
    updated: number;
  }[];
  restoringSnapshotId: string | null;
  onRestore: (snapshot: { instanceId: string; instanceName: string; id: string; label: string }) => void;
}) {
  return (
    <div className="rounded-xl border border-border bg-card p-4 space-y-2">
      <h4 className="font-semibold text-sm">Last Known Good</h4>
      <div className="space-y-1">
        {snapshots.slice(0, 3).map((s) => (
          <div key={s.id} className="flex items-center justify-between text-xs">
            <div>
              <p className="font-medium">{s.instanceName}: {s.label}</p>
              <p className="text-muted-foreground">
                {s.promotedAt ? new Date(s.promotedAt).toLocaleString() : 'Promotion time unavailable'}
                {' · '}{s.added} new · {s.removed} removed · {s.disabled} disabled · {s.updated} updated
              </p>
            </div>
            <button
              onClick={() => onRestore(s)}
              disabled={restoringSnapshotId !== null}
              className="text-primary hover:underline text-xs disabled:opacity-50"
            >
              {restoringSnapshotId === s.id ? 'Restoring…' : 'Restore'}
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}

function RecommendationsCard({
  hasInstances,
  hasCachedDb,
  loading,
  activeInstance,
  recommendations,
  onOpenMod,
  onBrowseMore,
}: {
  hasInstances: boolean;
  hasCachedDb: boolean;
  loading: boolean;
  activeInstance: InstanceRow | null;
  recommendations: RegistryItem[];
  onOpenMod: (itemId: string) => void;
  onBrowseMore: () => void;
}) {
  if (loading) return null;

  if (!hasInstances) {
    return (
      <div className="rounded-xl border border-dashed border-border bg-card p-6 text-center">
        <p className="text-muted-foreground">
          Once you have an instance, we&apos;ll show mods that work with it.
        </p>
        <button onClick={onBrowseMore} className="mt-3 rounded-lg border border-border px-4 py-2 text-sm font-medium hover:bg-accent">
          Browse all mods
        </button>
      </div>
    );
  }

  if (!hasCachedDb) {
    return (
      <div className="rounded-xl border border-dashed border-border bg-card p-6 text-center">
        <p className="text-muted-foreground">
          Download the registry to see compatible recommendations.
        </p>
      </div>
    );
  }

  if (recommendations.length === 0) {
    return (
      <div className="rounded-xl border border-dashed border-border bg-card p-6 text-center">
        <p className="text-muted-foreground">
          No new curated matches were found for {activeInstance?.name ?? 'this instance'}.
        </p>
        <button onClick={onBrowseMore} className="mt-3 rounded-lg border border-border px-4 py-2 text-sm font-medium hover:bg-accent">
          Browse catalog
        </button>
      </div>
    );
  }

  return (
    <div className="rounded-xl border border-border bg-card p-4 space-y-3">
      <div>
        <h4 className="font-semibold text-sm">Compatible recommendations</h4>
        <p className="text-xs text-muted-foreground">
          Ranked by category overlap with mods in {activeInstance?.name}, then filtered for MC {activeInstance?.minecraft_version} and {activeInstance?.loader}.
        </p>
      </div>
      <div className="grid gap-2 md:grid-cols-3">
        {recommendations.map((item) => (
          <button
            key={item.id}
            onClick={() => onOpenMod(item.id)}
            className="rounded-lg border border-border bg-muted p-3 text-left hover:bg-accent"
          >
            <span className="block text-sm font-medium">{item.name}</span>
            <span className="mt-1 block text-xs text-muted-foreground line-clamp-2">
              {item.description || `Curated ${item.content_type} from ${item.download_strategy}.`}
            </span>
            <span className="mt-2 block text-[11px] text-primary">
              {item.status === 'active' ? 'Curated and active' : item.status} · {item.download_strategy}
            </span>
          </button>
        ))}
      </div>
      <button onClick={onBrowseMore} className="rounded-lg border border-border px-4 py-2 text-sm font-medium hover:bg-accent">
        Browse more
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function timeSince(date: Date): string {
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const mins = Math.floor(diffMs / 60000);
  if (mins < 1) return 'Just now';
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  return date.toLocaleDateString();
}
