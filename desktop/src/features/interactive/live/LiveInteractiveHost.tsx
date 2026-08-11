/**
 * Live Interactive Host (High Interaction read surface).
 *
 * A High Interaction view of a single instance rendered through the shared
 * visuals. It runs the existing read-only commands, builds a fresh live scene
 * with a revision, and renders `LiveSceneView`. Every intent goes through
 * `routeLiveIntent`; an approved review route is forwarded as a DISCRIMINATED
 * `{ bridge, context }` to `onOpenStandardOperation`, which opens the STANDARD
 * surface — nothing executes a mutation here (remove uses the reviewed
 * InstallFlow on the Standard surface).
 *
 * SOL-2 gate (batch 2 / §17.3, §17.4):
 *  - Canonical state is PROJECTED at render over the BASE (unprojected) read
 *    scene, so busy/lock state is reversible (active -> idle clears busy) and
 *    always reflects the LATEST canonical values (no acceptance-time stale
 *    closures; a canonical change during an unresolved read is applied when
 *    the result lands).
 *  - Refresh marks the retained scene source `refreshing` (non-executable to
 *    `routeLiveIntent`) and the accepted read installs a new revision.
 *  - Requests use one monotonic generation (never reset) and verify the
 *    requested instance id before applying (latest-wins, instance-safe).
 *  - A dispatched review is recorded as an `in-review` proposal so the
 *    controller's duplicate gate coalesces; the next accepted read (terminal
 *    refresh) replaces the scene and clears the in-flight marker. Backend
 *    outcomes remain the Standard surface's authority.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  CapabilityFlags,
  VisualId,
  VisualInstance,
  VisualProposal,
  VisualScene,
} from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import { liveHighInteractionCapabilities } from './liveCapabilities';
import { nextRevision, liveSource } from './freshness';
import { assembleLiveScene, readLiveData, ok, err, type LiveReads } from './liveScene';
import { crashToVisual, runtimeToVisual, snapshotsToVisual } from './readAdapters';
import { routeLiveIntent } from './intentController';
import type { AvailabilityInput } from './intentController';
import type { LiveReviewRoute } from './operationBridges';
import type { LiveHostData } from './LiveSceneView';
import { LiveSceneView } from './LiveSceneView';
import { StatusChip } from '../visual/primitives/statusChips';

export type LiveHostLoad = (instanceId: string, revision: string) => Promise<LiveHostData>;

export function defaultLiveLoad(instanceId: string): Promise<LiveHostData> {
  return readLiveData(instanceId).then((reads) => buildHostData(reads));
}

/** Assemble `LiveHostData` from the fragment reads, preserving availability. */
export function buildHostData(reads: LiveReads): LiveHostData {
  const scene = assembleLiveScene(reads.detail.status === 'ok' ? reads.detail.value?.row.instance_id ?? '' : '', reads);
  return {
    scene,
    health: reads.health.status === 'ok' ? ok(true) : err<boolean>(),
    snapshots:
      reads.snapshots.status === 'ok'
        ? ok(snapshotsToVisual(reads.snapshots.value))
        : err<import('../domain/models').VisualSnapshot[]>(),
    crashEvidence:
      reads.investigation.status === 'ok'
        ? ok(reads.investigation.value ? crashToVisual(reads.investigation.value) : null)
        : err<import('../domain/models').VisualCrashEvidence | null>(),
    runtime:
      reads.detail.status === 'ok' && reads.detail.value && reads.memory.status === 'ok' && reads.javas.status === 'ok'
        ? ok(runtimeToVisual(reads.detail.value.row, reads.memory.value, reads.javas.value))
        : err<import('../domain/models').VisualRuntimeState | null>(),
  };
}

export type LiveHostState =
  | { kind: 'loading' }
  | { kind: 'scene'; instanceId: string; data: LiveHostData; refreshing: boolean }
  | { kind: 'error'; message: string };

/** Canonical app-level process state consumed by the host. */
export interface CanonicalProcessState {
  phase: string;
  instanceId?: string | null;
}

export interface LiveInteractiveHostProps {
  instanceId: string;
  onUseStandardView: () => void;
  onOpenStandardOperation?: (route: LiveReviewRoute) => void;
  load?: (instanceId: string, revision: string) => Promise<LiveHostData>;
  capabilities?: CapabilityFlags;
  reducedMotion?: boolean;
  /** Canonical app-level process state (useProcessController) when available. */
  processState?: CanonicalProcessState | null;
  /** True when a canonical install is ACTIVE (running) for this instance. */
  installActive?: boolean;
}

/** Map every canonical process phase conservatively onto the visual model. */
function projectLaunchState(phase: string): VisualInstance['launchState'] {
  switch (phase) {
    case 'launching':
    case 'starting':
      return 'starting';
    case 'running':
      return 'running';
    case 'stopping':
      return 'stopping';
    case 'delegated':
      return 'delegated';
    case 'failed':
      return 'failed';
    default:
      return 'idle';
  }
}

/** Canonical inputs needed to project a scene. */
export interface CanonicalInput {
  processState: CanonicalProcessState | null;
  installActive: boolean;
  instanceId: string;
}

/**
 * Pure, REVERSIBLE canonical projection over the BASE (unprojected) read
 * scene (SOL-2 §17.3). Busy is derived ONLY from canonical state; when the
 * canonical condition clears, the base read/player lock state is restored
 * (an idle -> launching -> idle transition never leaves the instance busy).
 */
export function projectCanonical(scene: VisualScene, canonical: CanonicalInput): VisualScene {
  if (!scene.instance) return scene;
  const isThisInstance = canonical.processState && canonical.processState.instanceId === canonical.instanceId;
  const launchState = isThisInstance
    ? projectLaunchState(canonical.processState?.phase ?? 'idle')
    : 'idle';
  const busy =
    (isThisInstance && canonical.processState?.phase !== 'idle' && canonical.processState?.phase !== 'failed')
    || canonical.installActive;
  const nextInstance: VisualInstance = {
    ...scene.instance,
    launchState,
    lockState: busy ? 'busy' : scene.instance.lockState,
  };
  return { ...scene, instance: nextInstance };
}

export function LiveInteractiveHost({
  instanceId,
  onUseStandardView,
  onOpenStandardOperation,
  load = defaultLiveLoad,
  capabilities = liveHighInteractionCapabilities(),
  reducedMotion = false,
  processState = null,
  installActive = false,
}: LiveInteractiveHostProps) {
  const [state, setState] = useState<LiveHostState>({ kind: 'loading' });
  const [selection, setSelection] = useState<VisualId | null>(null);
  const revisionRef = useRef(nextRevision());
  // Monotonic request generation — NEVER reset (instance switches must not
  // collide with in-flight request ids).
  const requestRef = useRef(0);
  const instanceIdRef = useRef(instanceId);
  instanceIdRef.current = instanceId;
  // A Standard review that is still open. It survives manual refreshes (a
  // refresh is NOT a review terminal event) and is cleared only when the host
  // unmounts (leaving High Interaction) or the instance changes.
  const reviewInFlightRef = useRef<VisualProposal | null>(null);

  const loadScene = useCallback(
    async (targetInstanceId: string, refreshing: boolean) => {
      const requestId = ++requestRef.current;
      const revision = revisionRef.current;
      if (!refreshing) setState({ kind: 'loading' });
      try {
        const data = await load(targetInstanceId, revision);
        if (requestId !== requestRef.current) return; // out-of-order result discarded
        if (instanceIdRef.current !== targetInstanceId) return; // instance switched
        if (!data.scene.instance) {
          setState({ kind: 'error', message: 'This instance has no readable state.' });
          return;
        }
        // Store the BASE (unprojected) read scene. Canonical state is projected
        // at render with the latest values — never captured in this closure.
        // If a Standard review is still open, preserve its in-flight marker
        // across this fresh read — a manual refresh is not a terminal event.
        const accepted = reviewInFlightRef.current
          ? { ...data, scene: { ...data.scene, proposals: [...data.scene.proposals, reviewInFlightRef.current] } }
          : data;
        // Bind the accepted scene to the instance it was loaded for (SOL-2
        // §19.4): a stale scene is withheld at render whenever the current
        // instanceId differs — never routed as the new target.
        setState({ kind: 'scene', instanceId: targetInstanceId, data: accepted, refreshing: false });
        setSelection((current) => {
          if (current === null) return null;
          const stillExists = data.scene.content.some((node) => node.id === current) || data.scene.instance?.id === current;
          return stillExists ? current : null;
        });
      } catch (error) {
        if (requestId !== requestRef.current) return;
        if (instanceIdRef.current !== targetInstanceId) return;
        setState({ kind: 'error', message: error instanceof Error ? error.message : 'Failed to load live data.' });
      }
    },
    [load],
  );

  // Instance-change reset is isolated: only instanceId triggers a reset+reload.
  // A review opened for the previous instance no longer applies.
  useEffect(() => {
    reviewInFlightRef.current = null;
    revisionRef.current = nextRevision();
    setSelection(null);
    void loadScene(instanceId, false);
  }, [instanceId, loadScene]);

  // Project the retained BASE scene with the CURRENT canonical values on every
  // render. Reversible, latest-wins, and immune to stale acceptance closures:
  // a canonical change while a read is unresolved is applied the moment the
  // accepted result lands (the projection always reads the newest props).
  const displayData = useMemo<LiveHostData | null>(() => {
    if (state.kind !== 'scene') return null;
    // Render-time identity guard (SOL-2 §19.4): the retained scene is bound to
    // the instance it was loaded for. If the current instanceId differs, the
    // old scene is withheld as loading/non-executable — React passive-effect
    // timing must never be a safety control.
    if (state.instanceId !== instanceId) return null;
    return {
      ...state.data,
      scene: projectCanonical(state.data.scene, { processState, installActive, instanceId }),
    };
  }, [state, processState, installActive, instanceId]);

  const refresh = useCallback(() => {
    revisionRef.current = nextRevision();
    // Keep the last scene visible but mark it refreshing (non-executable)
    // until the accepted read installs a new revision.
    setState((current) => {
      if (current.kind !== 'scene') return current;
      const scene: VisualScene = {
        ...current.data.scene,
        source: liveSource(revisionRef.current, 'refreshing'),
      };
      return { kind: 'scene', instanceId: current.instanceId, data: { ...current.data, scene }, refreshing: true };
    });
    void loadScene(instanceId, true);
  }, [loadScene, instanceId]);

  const handleIntent = useCallback(
    (intent: VisualIntent) => {
      const scene = displayData?.scene;
      const instance = displayData?.scene.instance;
      // Typed readiness/lock inputs (SOL-2 §18.3): player locks, pending/failed
      // recovery, active process/launch, and active installs each block review
      // with their own explanation. Selection/inspection stay available.
      const availability: AvailabilityInput = {
        locked: instance?.lockState === 'locked-by-player',
        recoveryBusy: instance?.recoveryReadiness === 'preparing' || instance?.recoveryReadiness === 'failed',
        processBusy:
          (instance?.launchState !== undefined && instance.launchState !== 'idle' && instance.launchState !== 'failed')
          || (instance?.lockState === 'busy' && !installActive),
        installBusy: installActive,
      };
      const route = routeLiveIntent(scene, intent, capabilities, instanceId, availability);
      if (route.status === 'selection') {
        if (intent.kind === 'select') {
          setSelection((current) => (current === intent.entityId ? null : intent.entityId));
        }
        return;
      }
      if (route.status === 'refresh-required') {
        refresh();
        return;
      }
      if (route.status === 'blocked') {
        return; // the visual should not have offered it; nothing executes
      }
      if (route.status === 'navigate') {
        return; // navigation to standard destinations is out of scope here
      }

      // review: record an in-review proposal so the controller's duplicate
      // gate coalesces. It is cleared only by leaving High Interaction (host
      // unmount) or an instance change — never by a manual refresh. Backend
      // outcomes stay in the Standard surface; nothing here claims a committed
      // result. The owning surface leaves High Interaction before working, so
      // re-entry always starts from a fresh read.
      let reviewRoute = route.route;
      if (state.kind === 'scene') {
        const proposal: VisualProposal = {
          id: `live:review:${reviewRoute.bridge}:${revisionRef.current}`,
          intent,
          phase: 'in-review',
          title: `Reviewing ${reviewRoute.bridge}`,
          summary: 'The Standard surface owns this review; it ends when you leave High Interaction.',
          destructive: false,
        };
        reviewInFlightRef.current = proposal;
        setState((current) =>
          current.kind === 'scene'
            ? { ...current, data: { ...current.data, scene: { ...current.data.scene, proposals: [...current.data.scene.proposals, proposal] } } }
            : current,
        );
      }

      // Enrich the install-flow context with the content's backend-derived
      // filename/kind resolved from the accepted live scene (never parsed from
      // a visual id). All other contexts pass through unchanged.
      const routeContext = reviewRoute.context;
      if (routeContext.kind === 'install-flow' && routeContext.contentId) {
        const node = displayData?.scene.content.find((n) => n.id === routeContext.contentId);
        if (node) {
          reviewRoute = {
            bridge: 'install-flow',
            context: {
              ...routeContext,
              // `name` may be a DERIVED display label; `fileLabel` holds the
              // authoritative on-disk filename whenever it is. The bridge must
              // always carry the real filename — the Standard surface resolves
              // the removal against the instance manifest with it.
              filename: node.fileLabel ?? node.name,
              contentKind: node.kind,
            },
          };
        }
      }
      onOpenStandardOperation?.(reviewRoute);
    },
    [state, displayData, capabilities, instanceId, installActive, onOpenStandardOperation, refresh],
  );

  return (
    <section aria-label="High Interaction view" className="space-y-3" data-testid="live-host">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-base font-bold text-foreground">High Interaction</h3>
        <div className="flex flex-wrap items-center gap-2">
          {state.kind === 'scene' && state.refreshing ? <StatusChip label="Refreshing…" /> : null}
          <button
            type="button"
            onClick={refresh}
            className="rounded-md border border-border bg-card px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
          >
            Refresh
          </button>
          <button
            type="button"
            onClick={onUseStandardView}
            className="rounded-md bg-primary px-3 py-1.5 text-sm font-semibold text-primary-foreground hover:opacity-90"
          >
            Use Standard view
          </button>
        </div>
      </div>

      {state.kind === 'loading' ? (
        <p className="text-sm text-muted-foreground" role="status">
          Loading live data…
        </p>
      ) : null}

      {state.kind === 'error' ? (
        <div className="rounded-xl border border-destructive/50 bg-destructive/5 p-4" role="alert">
          <p className="text-sm font-semibold text-destructive">Could not load live data.</p>
          <p className="mt-1 text-xs text-muted-foreground">{state.message}</p>
          <div className="mt-2 flex gap-2">
            <button
              type="button"
              onClick={refresh}
              className="rounded-md border border-border bg-card px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
            >
              Retry
            </button>
            <button
              type="button"
              onClick={onUseStandardView}
              className="rounded-md bg-primary px-3 py-1.5 text-sm font-semibold text-primary-foreground hover:opacity-90"
            >
              Use Standard view
            </button>
          </div>
        </div>
      ) : null}

      {displayData ? (
        <LiveSceneView
          data={displayData}
          capabilities={capabilities}
          selection={selection}
          onSelect={setSelection}
          onIntent={handleIntent}
          reducedMotion={reducedMotion}
        />
      ) : null}
    </section>
  );
}
