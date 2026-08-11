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
  getInstanceDetail,
  investigateInstanceEvidence,
  listInstances,
  listJavaRuntimes,
  listSnapshots,
  queryLaunchState,
  recommendInstanceMemory,
  type CrashInvestigation,
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

export interface LiveReads {
  detail: Fragment<InstanceDetail | null>;
  running: Fragment<RunningProcess | null>;
  health: Fragment<HealthReport | null>;
  snapshots: Fragment<Snapshot[]>;
  investigation: Fragment<CrashInvestigation | null>;
  memory: Fragment<MemoryRecommendation | null>;
  javas: Fragment<JavaRuntimeSummary[]>;
}

/** Loads every read needed for a live scene; each read failure is retained as an `error` fragment. */
export async function readLiveData(instanceId: string): Promise<LiveReads> {
  const safe = async <T>(run: () => Promise<T>): Promise<Fragment<T>> => {
    try {
      return ok(await run());
    } catch {
      return err<T>();
    }
  };
  const [detail, running, health, snapshots, investigation, memory, javas] = await Promise.all([
    safe(() => getInstanceDetail(instanceId)),
    safe(() => queryLaunchState()),
    safe(() => checkInstanceHealth(instanceId)),
    safe(() => listSnapshots(instanceId)),
    safe(() => investigateInstanceEvidence(instanceId)),
    safe(() => recommendInstanceMemory(instanceId)),
    safe(() => listJavaRuntimes()),
  ]);
  return { detail, running, health, snapshots, investigation, memory, javas };
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
export function assembleLiveScene(_instanceId: string, reads: LiveReads): VisualScene {
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
  const { content, relationships } = contentToVisual(detail, health, healthKnown);
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
