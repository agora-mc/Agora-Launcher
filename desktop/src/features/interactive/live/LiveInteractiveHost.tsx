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
import type { StandardDestination, VisualIntent } from '../domain/intents';
import type { InteractionPreference } from './presentationPreference';
import { liveHighInteractionCapabilities } from './liveCapabilities';
import { nextRevision, liveSource } from './freshness';
import {
  assembleLiveScene,
  pendingEnrichment,
  readEnrichmentData,
  readEssentialData,
  ok,
  err,
  type LiveReads,
} from './liveScene';
import { readContentDetail, readContentIcons, EMPTY_CONTENT_DETAIL, type ContentDetail } from './readAdapters';
import { crashToVisual, runtimeToVisual, snapshotsToVisual } from './readAdapters';
import { routeLiveIntent } from './intentController';
import type { AvailabilityInput } from './intentController';
import type { LiveReviewRoute } from './operationBridges';
import type { LiveHostData } from './LiveSceneView';
import { LiveSceneView } from './LiveSceneView';
import { StatusChip } from '../visual/primitives/statusChips';

/**
 * Load a scene, optionally painting a PARTIAL scene first.
 *
 * `onPartial` is an optional early-paint channel, not a second result: the
 * promise still resolves with the complete scene, and a loader that ignores
 * the callback behaves exactly as a single-phase loader always did.
 */
export type LiveHostLoad = (
  instanceId: string,
  revision: string,
  onPartial?: (data: LiveHostData) => void,
) => Promise<LiveHostData>;

/**
 * Two-phase default load.
 *
 * The instance read is cheap and the enrichment reads are not (a full health
 * scan, crash triage, jar dependency parsing, Java discovery). Awaiting all
 * eight before the first paint is what made High Interaction take seconds to
 * appear while the Standard editor — which blocks on `getInstanceDetail`
 * alone and streams the rest in — was effectively instant.
 *
 * Both phases start together, so the complete scene arrives no later than
 * before; the world is simply painted as soon as it CAN be. The partial scene
 * is stamped `refreshing`, which is literally true and already means
 * non-executable everywhere (`freshness.isExecutable`), so nothing can be
 * driven from half-read data.
 */
export function defaultLiveLoad(
  instanceId: string,
  revision: string = nextRevision(),
  onPartial?: (data: LiveHostData) => void,
): Promise<LiveHostData> {
  const enrichment = readEnrichmentData(instanceId);
  const essential = readEssentialData(instanceId);
  if (onPartial) {
    void essential.then((reads) => {
      const partial = buildHostData({ ...reads, ...pendingEnrichment() });
      if (!partial.scene.instance) return; // nothing paintable; wait for the full read
      onPartial({
        ...partial,
        scene: { ...partial.scene, source: liveSource(revision, 'refreshing') },
      });
    });
  }
  const icons = readContentIcons(instanceId);
  return Promise.all([essential, enrichment, icons]).then(([reads, extra, contentIcons]) =>
    buildHostData({ ...reads, ...extra }, contentIcons),
  );
}

/** Assemble `LiveHostData` from the fragment reads, preserving availability. */
export function buildHostData(reads: LiveReads, contentIcons: import('./readAdapters').ContentIcon[] = []): LiveHostData {
  const scene = assembleLiveScene(
    reads.detail.status === 'ok' ? reads.detail.value?.row.instance_id ?? '' : '',
    reads,
    contentIcons,
  );
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
  /**
   * `pending` marks a scene painted from the essential reads alone, with the
   * enrichment read still in flight. It is never a failure state — it exists
   * so the view can say "still checking" where an unresolved enrichment
   * fragment would otherwise read as "could not be verified".
   */
  | { kind: 'scene'; instanceId: string; data: LiveHostData; refreshing: boolean; pending: boolean }
  | { kind: 'error'; message: string };

/** Canonical app-level process state consumed by the host. */
export interface CanonicalProcessState {
  phase: string;
  instanceId?: string | null;
}

export interface LiveInteractiveHostProps {
  instanceId: string;
  onUseStandardView: () => void;
  /**
   * Rendered at the start of the host's control row (the page's Back button).
   *
   * Taking it as a slot rather than letting the page stack its own row above
   * keeps Back, Refresh and the Standard escape on ONE line, instead of two
   * rows of chrome above a view whose whole point is the view.
   */
  leading?: React.ReactNode;
  onOpenStandardOperation?: (route: LiveReviewRoute) => void;
  onNavigateStandard?: (destination: StandardDestination) => void;
  load?: LiveHostLoad;
  capabilities?: CapabilityFlags;
  reducedMotion?: boolean;
  /** The active presentation — `simple` renders the same useful structure
   * without the decorative flourish. */
  presentation?: InteractionPreference;
  /** Canonical app-level process state (useProcessController) when available. */
  processState?: CanonicalProcessState | null;
  /** True when a canonical install is ACTIVE (running) for this instance. */
  installActive?: boolean;
  /** Runs the real launch (the Standard launch flow) from the Play button. */
  onLaunch?: () => Promise<void> | void;
  /** Mirrors the Standard editor's complete launch gate, including local busy state. */
  launchAvailable?: boolean;
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

/**
 * Canonical phases that have RELEASED the instance.
 *
 * `exited` is the phase `useProcessController` sets when the backend's
 * `game-exited` event lands, and it is exactly as terminal as `idle` and
 * `failed` — the process is gone and the instance is playable again. Deriving
 * busy as "anything that is not idle or failed" left the instance locked
 * forever after a normal session, so the Play button never came back.
 *
 * Kept as a terminal ALLOWLIST rather than a busy list so the conservative
 * default survives: any phase this file has not seen still projects as busy
 * (matching `projectLaunchState`'s conservative mapping). The Standard editor
 * makes the same distinction with its blocking-phase list in
 * `pages/InstanceEditor.tsx` — that is why Standard's Play button recovers.
 */
const TERMINAL_PHASES = new Set(['idle', 'exited', 'failed']);

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
    (isThisInstance && !TERMINAL_PHASES.has(canonical.processState?.phase ?? 'idle'))
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
  leading,
  onOpenStandardOperation,
  onNavigateStandard,
  load = defaultLiveLoad,
  capabilities = liveHighInteractionCapabilities(),
  reducedMotion = false,
  presentation = 'high-interaction',
  processState = null,
  installActive = false,
  onLaunch,
  launchAvailable = true,
}: LiveInteractiveHostProps) {
  const [state, setState] = useState<LiveHostState>({ kind: 'loading' });
  const [selection, setSelection] = useState<VisualId | null>(null);
  /**
   * Catalogue detail for the SELECTED item only.
   *
   * Fetched here rather than in the view because the view sits in live/core,
   * which may not call Tauri — and fetched one at a time because enriching all
   * 130 nodes up front to fill a one-item panel is exactly the kind of per-mod
   * round trip that made the dependency read slow.
   */
  const [selectedDetail, setSelectedDetail] = useState<ContentDetail>(EMPTY_CONTENT_DETAIL);

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
      // Preserve an open Standard review's in-flight marker across a fresh
      // read — a refresh is not a review terminal event.
      const withReview = (data: LiveHostData): LiveHostData =>
        reviewInFlightRef.current
          ? { ...data, scene: { ...data.scene, proposals: [...data.scene.proposals, reviewInFlightRef.current] } }
          : data;
      try {
        const data = await load(targetInstanceId, revision, (partial) => {
          // The early paint passes the SAME generation and instance-identity
          // guards as the accepted result — an early frame must never be the
          // one place a stale or switched-away read gets through.
          if (requestId !== requestRef.current) return;
          if (instanceIdRef.current !== targetInstanceId) return;
          if (!partial.scene.instance) return;
          setState({
            kind: 'scene',
            instanceId: targetInstanceId,
            data: withReview(partial),
            refreshing: true,
            pending: true,
          });
        });
        if (requestId !== requestRef.current) return; // out-of-order result discarded
        if (instanceIdRef.current !== targetInstanceId) return; // instance switched
        if (!data.scene.instance) {
          setState({ kind: 'error', message: 'This instance has no readable state.' });
          return;
        }
        // Store the BASE (unprojected) read scene. Canonical state is projected
        // at render with the latest values — never captured in this closure.
        const accepted = withReview(data);
        // Bind the accepted scene to the instance it was loaded for (SOL-2
        // §19.4): a stale scene is withheld at render whenever the current
        // instanceId differs — never routed as the new target.
        setState({ kind: 'scene', instanceId: targetInstanceId, data: accepted, refreshing: false, pending: false });
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

  // Look up the selected item's catalogue entry. Stale responses are discarded
  // so a slow lookup can never overwrite a newer selection's detail.
  useEffect(() => {
    if (!selection) { setSelectedDetail(EMPTY_CONTENT_DETAIL); return; }
    const node = displayData?.scene.content.find((n) => n.id === selection);
    const ids = node?.catalogIds;
    if (!ids || (!ids.registryId && !ids.modrinthId && !ids.modJarId)) {
      setSelectedDetail(EMPTY_CONTENT_DETAIL);
      return;
    }
    let cancelled = false;
    setSelectedDetail(EMPTY_CONTENT_DETAIL);
    void readContentDetail(ids.registryId, ids.modrinthId, ids.modJarId ?? null).then((detail) => {
      if (!cancelled) setSelectedDetail(detail);
    });
    return () => { cancelled = true; };
  }, [selection, displayData]);

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
      return {
        kind: 'scene',
        instanceId: current.instanceId,
        data: { ...current.data, scene },
        refreshing: true,
        pending: current.pending,
      };
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
        // A partial scene is already waiting on its enrichment read — that IS
        // the pending re-read. Restarting it here would discard the in-flight
        // request and repaint from scratch for no new information.
        if (state.kind !== 'scene' || !state.pending) refresh();
        return;
      }
      if (route.status === 'blocked') {
        return; // the visual should not have offered it; nothing executes
      }
      if (route.status === 'navigate') {
        if (intent.kind === 'open-standard') onNavigateStandard?.(intent.destination);
        return; // Standard navigation is owned by the page host.
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
    [state, displayData, capabilities, instanceId, installActive, onOpenStandardOperation, onNavigateStandard, refresh],
  );

  return (
    <section
      aria-label={presentation === 'simple' ? 'Simple view' : 'High Interaction view'}
      className="space-y-3"
      data-testid="live-host"
      data-presentation={presentation}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        {/* No "High Interaction" heading: the surface announces itself, and the
            label was a row of chrome that said nothing the view did not. */}
        <div className="flex flex-wrap items-center gap-2">{leading}</div>
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
            Use Standard view (more features)
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
              Use Standard view (more features)
            </button>
          </div>
        </div>
      ) : null}

      {displayData ? (
        <LiveSceneView
          data={displayData}
          capabilities={capabilities}
          selection={selection}
          selectedDetail={selectedDetail}
          onSelect={setSelection}
          onIntent={handleIntent}
          onUseStandardView={onUseStandardView}
          onNavigateStandard={onNavigateStandard}
          onLaunch={onLaunch}
          launchAvailable={launchAvailable}
          reducedMotion={reducedMotion}
          presentation={presentation}
          pending={state.kind === 'scene' && state.pending}
        />
      ) : null}
    </section>
  );
}
