import { describe, expect, it, vi, beforeEach } from 'vitest';
import type {
  CrashInvestigation,
  HealthReport,
  InstanceDetail,
  InstanceRow,
  JavaRuntimeSummary,
  MemoryRecommendation,
  RunningProcess,
  Snapshot,
} from '@/lib/tauri';

const mocks = vi.hoisted(() => ({
  getInstanceDetail: vi.fn(),
  queryLaunchState: vi.fn(),
  checkInstanceHealth: vi.fn(),
  listSnapshots: vi.fn(),
  investigateInstanceEvidence: vi.fn(),
  recommendInstanceMemory: vi.fn(),
  listJavaRuntimes: vi.fn(),
  getDependencyGraph: vi.fn(),
  listInstances: vi.fn(),
  listInstanceContent: vi.fn(),
  enrichInstanceContent: vi.fn(),
}));

vi.mock('@/lib/tauri', () => ({
  getInstanceDetail: mocks.getInstanceDetail,
  queryLaunchState: mocks.queryLaunchState,
  checkInstanceHealth: mocks.checkInstanceHealth,
  listSnapshots: mocks.listSnapshots,
  investigateInstanceEvidence: mocks.investigateInstanceEvidence,
  recommendInstanceMemory: mocks.recommendInstanceMemory,
  listJavaRuntimes: mocks.listJavaRuntimes,
  getDependencyGraph: mocks.getDependencyGraph,
  listInstances: mocks.listInstances,
  listInstanceContent: mocks.listInstanceContent,
  enrichInstanceContent: mocks.enrichInstanceContent,
}));

import { defaultLiveLoad } from './LiveInteractiveHost';

function instanceRow(): InstanceRow {
  return {
    instance_id: 'inst-1',
    name: 'My World',
    minecraft_version: '1.20.1',
    loader: 'fabric',
    loader_version: '0.15.11',
    is_modpack: false,
    is_locked: false,
    last_launched_at: null,
    jvm_memory_mb: 2048,
    jvm_memory_mode: 'auto',
    jvm_gc: 'auto',
    jvm_custom_args: '',
    jvm_always_pre_touch: false,
    created_at: '2026-01-01T00:00:00Z',
    java_path: null,
    java_incompatible_override: false,
    icon_path: null,
    launch_mode_override: 'auto',
    import_source: null,
  };
}

function detail(): InstanceDetail {
  return {
    row: instanceRow(),
    manifest: {
      instance_id: 'inst-1',
      name: 'My World',
      created_from_pack: null,
      minecraft_version: '1.20.1',
      loader: 'fabric',
      loader_version: '0.15.11',
      is_locked: false,
      mods: [],
      resourcepacks: [],
      shaders: [],
      datapacks: [],
      worlds: [],
      user_preferences: {},
    },
    snapshot_readiness: 'ready',
    snapshot_error: null,
  };
}

const health: HealthReport = {
  score: 'green',
  scan_token: 't',
  blockers: [],
  warnings: [],
  recommendations: [],
};

const snapshot: Snapshot = {
  id: 's1',
  label: 'Before change',
  created_at: '2026-01-01T00:00:00Z',
  file_count: 0,
  size_estimate: 4 * 1024 * 1024,
  is_lkg: false,
  is_current_lkg: false,
  is_pre_restore: false,
};

const investigation: CrashInvestigation = {
  evidence: {
    sources: [],
    primary_index: 0,
    aggregate_bytes: 0,
    any_truncated: false,
    any_stale: false,
    failure_category: 'NoEvidence',
  },
  fingerprint: null,
  triage: { matched: false, signature_name: null, solution_markdown: null, action_button_json: null },
  suspects: [],
  failure_category: 'NoEvidence',
};

const memory: MemoryRecommendation = {
  recommended_mb: 4096,
  tier_label: '4 GB',
  tier_index: 1,
  is_large_resource_pack_adjustment: false,
  ram_capped: false,
  insufficient_system_ram: false,
  system_ram_mb: 16384,
  next_tier_mb: 8192,
  next_tier_label: '8 GB',
  factors: [],
  explanation: 'more ram',
};

const javas: JavaRuntimeSummary[] = [
  { path: '/jdk', version: 17, version_string: '17', source: 'managed', arch: 'amd64' },
];

const running: RunningProcess[] = [];

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getInstanceDetail.mockResolvedValue(detail());
  mocks.queryLaunchState.mockResolvedValue(running);
  mocks.checkInstanceHealth.mockResolvedValue(health);
  mocks.listSnapshots.mockResolvedValue([snapshot]);
  mocks.investigateInstanceEvidence.mockResolvedValue(investigation);
  mocks.recommendInstanceMemory.mockResolvedValue(memory);
  mocks.listJavaRuntimes.mockResolvedValue(javas);
  mocks.getDependencyGraph.mockResolvedValue([]);
  mocks.listInstanceContent.mockResolvedValue([]);
  mocks.enrichInstanceContent.mockResolvedValue([]);
});

describe('live default loader failure semantics (SOL-2 BLOCKER 4/D)', () => {
  it('healthy default load produces a fresh scene with all fragments ok', async () => {
    const data = await defaultLiveLoad('inst-1');
    expect(data.scene.instance?.name).toBe('My World');
    expect(data.scene.source.kind).toBe('live');
    if (data.scene.source.kind === 'live') expect(data.scene.source.freshness).toBe('fresh');
    expect(data.health.status).toBe('ok');
    expect(data.snapshots.status).toBe('ok');
    expect(data.crashEvidence.status).toBe('ok');
    expect(data.runtime.status).toBe('ok');
  });

  it('getInstanceDetail failure yields an empty scene (host reports error, never a valid empty instance)', async () => {
    mocks.getInstanceDetail.mockRejectedValue(new Error('db down'));
    const data = await defaultLiveLoad('inst-1');
    expect(data.scene.instance).toBeUndefined();
    if (data.scene.source.kind === 'live') expect(data.scene.source.freshness).toBe('unknown');
  });

  it('checkInstanceHealth failure keeps health unavailable (never treated as ready)', async () => {
    mocks.checkInstanceHealth.mockRejectedValue(new Error('health down'));
    const data = await defaultLiveLoad('inst-1');
    expect(data.health.status).toBe('error');
    expect(data.scene.findings).toEqual([]);
    if (data.scene.source.kind === 'live') expect(data.scene.source.freshness).toBe('unknown'); // aggregate non-executable
  });

  it('listSnapshots failure keeps the snapshot timeline unavailable', async () => {
    mocks.listSnapshots.mockRejectedValue(new Error('snapshots down'));
    const data = await defaultLiveLoad('inst-1');
    expect(data.snapshots.status).toBe('error');
  });

  it('investigateInstanceEvidence failure keeps crash evidence unavailable', async () => {
    mocks.investigateInstanceEvidence.mockRejectedValue(new Error('evidence down'));
    const data = await defaultLiveLoad('inst-1');
    expect(data.crashEvidence.status).toBe('error');
  });

  it('recommendInstanceMemory failure makes runtime visibly unavailable', async () => {
    mocks.recommendInstanceMemory.mockRejectedValue(new Error('memory down'));
    const data = await defaultLiveLoad('inst-1');
    expect(data.runtime.status).toBe('error');
  });

  it('listJavaRuntimes failure makes runtime visibly unavailable', async () => {
    mocks.listJavaRuntimes.mockRejectedValue(new Error('javas down'));
    const data = await defaultLiveLoad('inst-1');
    expect(data.runtime.status).toBe('error');
  });

  it('queryLaunchState failure keeps the instance visible but process-uncertain (busy, aggregate unknown)', async () => {
    mocks.queryLaunchState.mockRejectedValue(new Error('launch state down'));
    const data = await defaultLiveLoad('inst-1');
    expect(data.scene.instance?.name).toBe('My World');
    // Process state is unknown -> conservatively busy, never editable.
    expect(data.scene.instance?.lockState).toBe('busy');
    if (data.scene.source.kind === 'live') expect(data.scene.source.freshness).toBe('unknown');
  });

  it('all default commands failing yields an empty unknown scene with every fragment error', async () => {
    mocks.getInstanceDetail.mockRejectedValue(new Error('x'));
    mocks.queryLaunchState.mockRejectedValue(new Error('x'));
    mocks.checkInstanceHealth.mockRejectedValue(new Error('x'));
    mocks.listSnapshots.mockRejectedValue(new Error('x'));
    mocks.investigateInstanceEvidence.mockRejectedValue(new Error('x'));
    mocks.recommendInstanceMemory.mockRejectedValue(new Error('x'));
    mocks.listJavaRuntimes.mockRejectedValue(new Error('x'));
    const data = await defaultLiveLoad('inst-1');
    expect(data.scene.instance).toBeUndefined();
    if (data.scene.source.kind === 'live') expect(data.scene.source.freshness).toBe('unknown');
    expect(data.health.status).toBe('error');
    expect(data.snapshots.status).toBe('error');
    expect(data.crashEvidence.status).toBe('error');
    expect(data.runtime.status).toBe('error');
  });
});

describe('live default loader two-phase paint', () => {
  /** Hold a read open so the enrichment phase cannot resolve early. */
  function deferred<T>() {
    let resolve: (value: T) => void = () => undefined;
    const promise = new Promise<T>((r) => { resolve = r; });
    return { promise, resolve };
  }

  it('emits a partial scene from the essential reads before the enrichment lands', async () => {
    const slowHealth = deferred<HealthReport>();
    mocks.checkInstanceHealth.mockReturnValue(slowHealth.promise);

    const partials: Array<{ name?: string; freshness?: string }> = [];
    const complete = defaultLiveLoad('inst-1', 'rev-1', (partial) => {
      partials.push({
        name: partial.scene.instance?.name,
        freshness: partial.scene.source.kind === 'live' ? partial.scene.source.freshness : undefined,
      });
    });

    // Let the essential reads settle while health is still outstanding.
    await new Promise((r) => setTimeout(r, 0));

    expect(partials).toHaveLength(1);
    expect(partials[0].name).toBe('My World');
    // Non-executable until the complete read installs a fresh revision — a
    // half-read scene must never drive a review.
    expect(partials[0].freshness).toBe('refreshing');

    slowHealth.resolve(health);
    const data = await complete;
    expect(data.health.status).toBe('ok');
    if (data.scene.source.kind === 'live') expect(data.scene.source.freshness).toBe('fresh');
  });

  it('starts the enrichment reads immediately, not after the essential phase', async () => {
    const complete = defaultLiveLoad('inst-1', 'rev-1', () => undefined);
    // Both phases are dispatched in the same tick, so a complete scene is
    // never slower than the old single Promise.all.
    expect(mocks.getInstanceDetail).toHaveBeenCalledTimes(1);
    expect(mocks.checkInstanceHealth).toHaveBeenCalledTimes(1);
    expect(mocks.getDependencyGraph).toHaveBeenCalledTimes(1);
    await complete;
  });

  it('skips the partial paint when the instance is unreadable', async () => {
    mocks.getInstanceDetail.mockResolvedValue(null);
    const onPartial = vi.fn();
    await defaultLiveLoad('inst-1', 'rev-1', onPartial);
    // Nothing paintable — the host reports its error state from the complete
    // read rather than flashing an empty world first.
    expect(onPartial).not.toHaveBeenCalled();
  });
});

describe('readEssentialData session narrowing', () => {
  function session(instanceId: string, sessionId: number): RunningProcess {
    return { instance_id: instanceId, pid: 1000 + sessionId, session_id: sessionId };
  }

  // The backend reports every live session now. A scene depicts one instance,
  // so another instance being up must not make this one look busy.
  it('ignores sessions belonging to other instances', async () => {
    const { readEssentialData } = await import('./liveScene');
    mocks.queryLaunchState.mockResolvedValue([session('other-inst', 1)]);
    const reads = await readEssentialData('inst-1');
    expect(reads.running.status).toBe('ok');
    expect(reads.running.status === 'ok' && reads.running.value).toBeNull();
  });

  it('reports this instance session when it is the one running', async () => {
    const { readEssentialData } = await import('./liveScene');
    mocks.queryLaunchState.mockResolvedValue([session('other-inst', 1), session('inst-1', 2)]);
    const reads = await readEssentialData('inst-1');
    expect(reads.running.status === 'ok' && reads.running.value?.session_id).toBe(2);
  });

  it('keeps a failed read as an error fragment rather than a null process', async () => {
    const { readEssentialData } = await import('./liveScene');
    mocks.queryLaunchState.mockRejectedValue(new Error('down'));
    const reads = await readEssentialData('inst-1');
    expect(reads.running.status).not.toBe('ok');
  });
});
