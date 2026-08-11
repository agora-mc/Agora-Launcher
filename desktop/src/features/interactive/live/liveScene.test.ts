import { describe, expect, it } from 'vitest';
import { allReadsOk, assembleLiveScene, derivedFreshness, err, ok, type LiveReads } from './liveScene';
import { instanceToVisual, contentToVisual } from './readAdapters';
import type { InstanceDetail, InstanceRow, RunningProcess } from '@/lib/tauri';

function instanceRow(overrides: Partial<InstanceRow> = {}): InstanceRow {
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
    ...overrides,
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
      mods: [
        { filename: 'sodium.jar', registry_id: 'sodium', modrinth_id: null, source: 'registry', version: '0.5', sha256: 'abc', installed_at: '', enabled: true, content_type: 'mod', mod_jar_id: null },
      ],
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

function reads(overrides: Partial<LiveReads> = {}): LiveReads {
  return {
    detail: ok<InstanceDetail | null>(detail()),
    running: ok<RunningProcess | null>(null),
    health: ok(null),
    snapshots: ok([]),
    investigation: ok(null),
    memory: ok(null),
    javas: ok([]),
    ...overrides,
  };
}

describe('live scene fragments (SOL-2 BLOCKER 1)', () => {
  it('any failed read makes the aggregate freshness unknown (never fresh)', () => {
    expect(allReadsOk(reads())).toBe(true);
    expect(derivedFreshness(reads())).toBe('fresh');
    expect(derivedFreshness(reads({ health: err() }))).toBe('unknown');
    expect(derivedFreshness(reads({ snapshots: err() }))).toBe('unknown');
    expect(derivedFreshness(reads({ running: err() }))).toBe('unknown');
    expect(derivedFreshness(reads({ detail: err() }))).toBe('unknown');
  });

  it('an instance read failure yields an empty scene (host reports error, not a valid instance)', () => {
    const scene = assembleLiveScene('inst-1', reads({ detail: err() }));
    expect(scene.instance).toBeUndefined();
    expect(scene.content).toHaveLength(0);
    if (scene.source.kind === 'live') expect(scene.source.freshness).toBe('unknown');
  });

  it('a health read failure produces no findings and no false "healthy" nodes', () => {
    const scene = assembleLiveScene('inst-1', reads({ health: err() }));
    if (scene.source.kind === 'live') expect(scene.source.freshness).toBe('unknown');
    expect(scene.findings).toHaveLength(0);
    expect(scene.content.length).toBeGreaterThan(0);
    expect(scene.content.every((node) => node.health === 'unknown')).toBe(true);
  });

  it('process-state uncertainty is conservative (busy, never editable)', () => {
    const visual = instanceToVisual(detail(), null, true);
    expect(visual.lockState).toBe('busy');
  });

  it('content nodes are marked healthy only when a health report was read', () => {
    const { content: withHealth } = contentToVisual(detail(), null, true);
    expect(withHealth[0].health).toBe('healthy');
    const { content: withoutHealth } = contentToVisual(detail(), null, false);
    expect(withoutHealth[0].health).toBe('unknown');
  });
});
