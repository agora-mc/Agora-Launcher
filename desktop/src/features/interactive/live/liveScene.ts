/**
 * Live scene loader: run the read-only commands and assemble a `VisualScene`.
 *
 * Read-only by construction — no mutation command is ever invoked here. Every
 * read returns a `Fragment`: `ok` with a value, or `error`. A failing read is
 * never erased into an ordinary empty value — it stays `error` and is rendered
 * as unavailable/unknown (FIX BEFORE LIVE MODE 1 / SOL-2 BLOCKER 1). Aggregate
 * scene freshness is derived from the fragments: any `error` makes the scene
 * non-executable (`unknown`), and an error instance read surfaces as a
 * host-level error (never a valid empty instance).
 */

import {
  checkInstanceHealth,
  getDependencyGraph,
  getInstanceDetail,
  investigateInstanceEvidence,
  listInstances,
  listJavaRuntimes,
  listSnapshots,
  queryLaunchState,
  recommendInstanceMemory,
  type CrashInvestigation,
  type DependencyEdge,
  type HealthReport,
  type InstanceDetail,
  type InstanceRow,
  type JavaRuntimeSummary,
  type MemoryRecommendation,
  type RunningProcess,
  type Snapshot,
} from '@/lib/tauri';
import type { LiveFreshness } from './freshness';
import { liveSource, nextRevision } from './freshness';
import {
  contentToVisual,
  healthToVisual,
  instanceToVisual,
  type ContentIcon,
} from './readAdapters';
import type { VisualScene } from '../domain/models';

/** A single read: `ok` carries the value; `error` means the read failed. */
export type Fragment<T> = { status: 'ok'; value: T } | { status: 'error' };

export function ok<T>(value: T): Fragment<T> {
  return { status: 'ok', value };
}

export function err<T>(): Fragment<T> {
  return { status: 'error' };
}

/**
 * The reads that PAINT the world: instance identity and process state.
 *
 * Both are cheap (a local-state row plus the manifest, and the in-memory
 * launch record), which is what makes a first paint possible before the
 * expensive reads land.
 */
export interface EssentialReads {
  detail: Fragment<InstanceDetail | null>;
  running: Fragment<RunningProcess | null>;
}

/**
 * The reads that ENRICH an already-painted world.
 *
 * These are the expensive ones — a full health scan, crash-evidence triage,
 * jar dependency parsing, Java discovery — and none of them are needed to
 * show the instance, its shelf, or the Play button.
 */
export interface EnrichmentReads {
  health: Fragment<HealthReport | null>;
  snapshots: Fragment<Snapshot[]>;
  investigation: Fragment<CrashInvestigation | null>;
  memory: Fragment<MemoryRecommendation | null>;
  javas: Fragment<JavaRuntimeSummary[]>;
  /** Mod-to-mod dependency edges; empty when the read failed. */
  dependencies: Fragment<DependencyEdge[]>;
}

export type LiveReads = EssentialReads & EnrichmentReads;

/** Run a read, keeping a failure as an `error` fragment rather than throwing. */
async function safe<T>(run: () => Promise<T>): Promise<Fragment<T>> {
  try {
    return ok(await run());
  } catch {
    return err<T>();
  }
}

/** Instance identity and process state — enough to paint the world. */
export async function readEssentialData(instanceId: string): Promise<EssentialReads> {
  const [detail, sessions] = await Promise.all([
    safe(() => getInstanceDetail(instanceId)),
    safe(() => queryLaunchState()),
  ]);
  // The backend reports every live session; this scene only depicts one
  // instance, so narrow to that instance's session. Another instance running
  // must not make this one look busy.
  const running: Fragment<RunningProcess | null> = sessions.status === 'ok'
    ? ok(sessions.value.find((session) => session.instance_id === instanceId) ?? null)
    : err<RunningProcess | null>();
  return { detail, running };
}

/** The expensive reads that enrich an already-painted world. */
export async function readEnrichmentData(instanceId: string): Promise<EnrichmentReads> {
  const [health, snapshots, investigation, memory, javas, dependencies] = await Promise.all([
    safe(() => checkInstanceHealth(instanceId)),
    safe(() => listSnapshots(instanceId)),
    safe(() => investigateInstanceEvidence(instanceId)),
    safe(() => recommendInstanceMemory(instanceId)),
    safe(() => listJavaRuntimes()),
    safe(() => getDependencyGraph(instanceId)),
  ]);
  return { health, snapshots, investigation, memory, javas, dependencies };
}

/**
 * Enrichment fragments for a scene whose enrichment read is still IN FLIGHT.
 *
 * They are `error` because that is what the fragment model means by "no
 * value": every consumer already renders them as unavailable/unknown, and a
 * partial scene is never `fresh`, so nothing becomes executable on their
 * account. The host carries a separate `pending` flag for the one place the
 * difference is user-visible — "still checking" must not read as "failed".
 */
export function pendingEnrichment(): EnrichmentReads {
  return {
    health: err<HealthReport | null>(),
    snapshots: err<Snapshot[]>(),
    investigation: err<CrashInvestigation | null>(),
    memory: err<MemoryRecommendation | null>(),
    javas: err<JavaRuntimeSummary[]>(),
    dependencies: err<DependencyEdge[]>(),
  };
}

/**
 * Loads every read needed for a complete live scene; each read failure is
 * retained as an `error` fragment.
 *
 * Both phases start together, so the total time to a COMPLETE scene is
 * unchanged — the split exists so a caller can paint after the essential
 * half instead of waiting for the slowest of all eight.
 */
export async function readLiveData(instanceId: string): Promise<LiveReads> {
  const [essential, enrichment] = await Promise.all([
    readEssentialData(instanceId),
    readEnrichmentData(instanceId),
  ]);
  return { ...essential, ...enrichment };
}

/** True when every relevant fragment read succeeded. */
export function allReadsOk(reads: LiveReads): boolean {
  return (
    reads.detail.status === 'ok'
    && reads.running.status === 'ok'
    && reads.health.status === 'ok'
    && reads.snapshots.status === 'ok'
    && reads.investigation.status === 'ok'
    && reads.memory.status === 'ok'
    && reads.javas.status === 'ok'
    && reads.dependencies.status === 'ok'
  );
}

/**
 * Derive the aggregate scene freshness from the fragments. Any read failure
 * makes the whole scene non-executable (`unknown`) — a partially-failed read
 * is never relabelled `fresh` (SOL-2 BLOCKER 1).
 */
export function derivedFreshness(reads: LiveReads): LiveFreshness {
  return allReadsOk(reads) ? 'fresh' : 'unknown';
}

/**
 * Assembles a live scene from the read data. The scene source freshness is
 * derived from the fragments, never blindly set to `fresh`. The instance must
 * have been read successfully; otherwise the scene is left empty and the HOST
 * reports an error state (this function's caller checks `reads.detail`).
 */
export function assembleLiveScene(
  _instanceId: string,
  reads: LiveReads,
  contentIcons: ContentIcon[] = [],
): VisualScene {
  const revision = nextRevision();
  const source = liveSource(revision, derivedFreshness(reads));
  if (reads.detail.status !== 'ok' || !reads.detail.value) {
    return { source, content: [], relationships: [], findings: [], proposals: [] };
  }
  const detail = reads.detail.value;
  const running = reads.running.status === 'ok' ? reads.running.value : null;
  const health = reads.health.status === 'ok' ? reads.health.value : null;
  const healthKnown = reads.health.status === 'ok';
  const instance = instanceToVisual(detail, running, reads.running.status !== 'ok');
  const dependencies = reads.dependencies.status === 'ok' ? reads.dependencies.value : [];
  const { content, relationships } = contentToVisual(detail, health, healthKnown, dependencies, contentIcons);
  return {
    source,
    instance,
    content,
    relationships,
    findings: healthKnown && health ? healthToVisual(health) : [],
    proposals: [],
  };
}

/** Resolve a single instance row by id (used to confirm the instance exists). */
export async function findInstanceRow(instanceId: string): Promise<InstanceRow | null> {
  try {
    const rows = await listInstances();
    return rows.find((row) => row.instance_id === instanceId) ?? null;
  } catch {
    return null;
  }
}
