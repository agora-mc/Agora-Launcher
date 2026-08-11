import { describe, expect, it } from 'vitest';
import {
  contentToVisual,
  crashToVisual,
  displayNameFromFilename,
  healthToVisual,
  instanceToVisual,
  runtimeToVisual,
  snapshotsToVisual,
} from './readAdapters';
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

function detail(overrides: Partial<InstanceDetail> = {}): InstanceDetail {
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
        { filename: 'sodium.jar', registry_id: 'sodium', modrinth_id: null, source: 'registry', version: '0.5', sha256: 'abc123', installed_at: '', enabled: true, content_type: 'mod', mod_jar_id: null },
        { filename: 'tweakeroo.jar', registry_id: 'tweakeroo', modrinth_id: null, source: 'registry', version: null, sha256: 'def456', installed_at: '', enabled: false, content_type: 'mod', mod_jar_id: null },
      ],
      resourcepacks: [],
      shaders: [],
      datapacks: [],
      worlds: [],
      user_preferences: {},
    },
    snapshot_readiness: 'ready',
    snapshot_error: null,
    ...overrides,
  };
}

const health: HealthReport = {
  score: 'yellow',
  scan_token: 'super-secret-token',
  blockers: [
    {
      kind: 'loader_requirement',
      mod_id: 'sodium',
      filename: 'sodium.jar',
      message: 'Loader does not fit',
      suggested_action: 'Choose a compatible loader',
      loader_compatibility: {
        loader: 'fabric',
        current_version: '0.14',
        recommended_version: '0.15.11',
        compatible_versions: ['0.15.11'],
        requirements: [
          {
            declaring_mod_id: null,
            declaring_mod_ids: ['sodium'],
            target_id: 'fabric-language-kotlin',
            version_ranges: [],
            importance: 'required',
            candidate_version: null,
            verdict: 'unsatisfied',
          },
        ],
        conflicts: [],
      },
    },
  ],
  warnings: [],
  recommendations: [{ kind: 'memory', mod_id: null, source_filename: null, message: 'Use automatic memory', suggested_action: 'Keep automatic' }],
};

const investigation: CrashInvestigation = {
  evidence: {
    sources: [
      {
        meta: { basename: 'crash-2026-08-09.txt', kind: 'CrashReport', size_bytes: 100, truncated: false, stale: false, supplementary: false, modified_at: null, line_count: 10 },
        text: 'FULL SECRET LOG CONTENTS THAT MUST NOT LEAK',
      },
    ],
    primary_index: 0,
    aggregate_bytes: 100,
    any_truncated: false,
    any_stale: false,
    failure_category: 'CrashReport',
  },
  fingerprint: { exception_class: 'java.lang.OutOfMemoryError', top_frames: ['a.b.C'] },
  triage: { matched: true, signature_name: 'oom', solution_markdown: null, action_button_json: null },
  suspects: [
    { mod_id: 'sodium', filename: 'sodium.jar', total_score: 5, breakdown: { secret: 'internal' }, is_dependent_of: null },
  ],
  failure_category: 'CrashReport',
};

describe('live readAdapters — DTO redaction (SOL-2 §14.3.6)', () => {
  it('instanceToVisual never exposes paths, process handles, or raw config', () => {
    const visual = instanceToVisual(detail(), null);
    expect(JSON.stringify(visual)).not.toMatch(/java_path|icon_path|sha256|manifest|jvm_/);
    expect(visual.id).toBe('inst-1');
    expect(visual.contentSummary).toEqual({ enabled: 1, disabled: 1, needsAttention: 0 });
  });

  it('instanceToVisual preserves lock/launch/recovery state from DTOs', () => {
    const running: RunningProcess = { instance_id: 'inst-1', pid: 42, session_id: 1 };
    expect(instanceToVisual(detail({ row: instanceRow({ is_locked: true }) }), null).lockState).toBe('locked-by-player');
    expect(instanceToVisual(detail(), running).launchState).toBe('running');
    expect(instanceToVisual(detail(), running).lockState).toBe('busy');
    expect(instanceToVisual(detail({ snapshot_readiness: 'pending' }), null).recoveryReadiness).toBe('preparing');
  });

  it('contentToVisual maps nodes and relationships without leaking sha256/resolved paths', () => {
    const { content, relationships } = contentToVisual(detail(), health);
    expect(JSON.stringify(content)).not.toMatch(/sha256|resolved_path|source_url/);
    expect(content).toHaveLength(2);
    expect(relationships.length).toBeGreaterThanOrEqual(1);
    const rel = relationships[0];
    expect(rel.kind).toBe('requires');
    expect(rel.state).toBe('missing');
    expect(rel.explanation).toContain('fabric-language-kotlin');
  });

  it('healthToVisual maps severity and never includes the scan token', () => {
    const findings = healthToVisual(health);
    expect(JSON.stringify(findings)).not.toContain('scan_token');
    expect(JSON.stringify(findings)).not.toContain('super-secret-token');
    expect(findings.some((f) => f.severity === 'blocker' && f.structuredKind === 'loader-compatibility')).toBe(true);
    expect(findings.some((f) => f.severity === 'recommendation')).toBe(true);
  });

  it('snapshotsToVisual adds an authoritative sortKey and never leaks object metadata', () => {
    const rows: Snapshot[] = [
      { id: 's1', label: null, created_at: '2026-08-08T10:00:00Z', file_count: 100, size_estimate: 5 * 1024 * 1024, is_lkg: true, is_current_lkg: false, is_pre_restore: false },
      { id: 's2', label: 'Manual', created_at: '2026-08-09T10:00:00Z', file_count: 200, size_estimate: 200 * 1024 * 1024, is_lkg: false, is_current_lkg: false, is_pre_restore: true },
    ];
    const visuals = snapshotsToVisual(rows);
    expect(visuals[0].sortKey).toBe('2026-08-08T10:00:00Z');
    expect(visuals[1].role).toBe('undo-restore');
    expect(JSON.stringify(visuals)).not.toMatch(/file_count/);
    expect(visuals[1].sizeLabel).toContain('MB');
  });

  it('crashToVisual redacts full logs, fingerprints, and suspect breakdowns', () => {
    const visual = crashToVisual(investigation);
    expect(JSON.stringify(visual)).not.toContain('FULL SECRET LOG');
    expect(JSON.stringify(visual)).not.toMatch(/top_frames|breakdown|fingerprint|failure_category/);
    expect(visual.hypotheses[0].title).toContain('sodium.jar');
    // Recovery is NOT claimed ready at read time (SOL-2 BLOCKER D).
    expect(visual.experiment.recoveryReady).toBe(false);
    expect(visual.privacyNote).toContain('stays on this device');
  });

  it('runtimeToVisual preserves current vs recommended memory and redacts paths', () => {
    const memory: MemoryRecommendation = {
      recommended_mb: 4096,
      tier_label: 'Recommended tier',
      tier_index: 2,
      is_large_resource_pack_adjustment: false,
      ram_capped: false,
      insufficient_system_ram: false,
      system_ram_mb: 16384,
      next_tier_mb: 6144,
      next_tier_label: 'Next tier',
      factors: [],
      explanation: 'Automatic is recommended.',
    };
    const javas: JavaRuntimeSummary[] = [{ path: '/opt/java17', version: 17, version_string: '17', source: 'managed', arch: null }];
    const visual = runtimeToVisual(instanceRow({ java_path: '/opt/java17', jvm_memory_mode: 'manual', jvm_memory_mb: 2048 }), memory, javas);
    expect(JSON.stringify(visual)).not.toMatch(/java_path|version_string/);
    expect(visual.memory.mode.current).toBe('manual');
    expect(visual.memory.recommendedMiB).toBe(4096);
    expect(visual.runtime.managedByAgora).toBe(true);
  });
});

/**
 * SOL §22.2 / §22.6 — the null-identity path.
 *
 * Every other fixture in this suite carries populated `mod_id`/`registry_id`.
 * That is exactly why a `null === null` identity match survived five Sol gates,
 * 243 e2e tests, and a live UX review: the fail-closed contract was tested for
 * read FAILURE but never for read AMBIGUITY. These tests are mandatory.
 */
describe('live readAdapters — unattributed findings and null identities (SOL §22.2)', () => {
  const uncuratedDetail = () =>
    detail({
      manifest: {
        instance_id: 'inst-1',
        name: 'My World',
        created_from_pack: null,
        minecraft_version: '1.20.1',
        loader: 'fabric',
        loader_version: '0.15.11',
        is_locked: false,
        // Locally imported mods: no registry identity at all.
        mods: [
          { filename: 'local-a.jar', registry_id: null, modrinth_id: null, source: 'local', version: null, sha256: 'a', installed_at: '', enabled: true, content_type: 'mod', mod_jar_id: null },
          { filename: 'local-b.jar', registry_id: null, modrinth_id: null, source: 'local', version: null, sha256: 'b', installed_at: '', enabled: true, content_type: 'mod', mod_jar_id: null },
        ],
        resourcepacks: [],
        shaders: [],
        datapacks: [],
        worlds: [],
        user_preferences: {},
      },
    });

  const instanceLevelWarning: HealthReport = {
    score: 'yellow',
    scan_token: 't',
    blockers: [],
    warnings: [
      {
        kind: 'manifest_drift',
        mod_id: null,
        filename: null,
        message: 'The instance manifest tracks 1 enabled mod file(s) that are absent from mods/.',
        suggested_action: 'Repair or reinstall the modpack.',
      },
    ],
    recommendations: [],
  };

  it('an instance-level WARNING never marks content nodes (T6-11)', () => {
    const { content } = contentToVisual(uncuratedDetail(), instanceLevelWarning);
    expect(content).toHaveLength(2);
    for (const node of content) expect(node.health).toBe('healthy');
  });

  it('an instance-level BLOCKER never marks content nodes (the more dangerous arm)', () => {
    const report: HealthReport = {
      ...instanceLevelWarning,
      warnings: [],
      blockers: [
        { kind: 'manifest_drift', mod_id: null, filename: null, message: 'Instance-level blocker', suggested_action: null },
      ],
    };
    const { content } = contentToVisual(uncuratedDetail(), report);
    for (const node of content) expect(node.health).toBe('healthy');
  });

  it('still attributes a finding that DOES name a file', () => {
    const report: HealthReport = {
      ...instanceLevelWarning,
      warnings: [
        { kind: 'bad_mod', mod_id: null, filename: 'local-a.jar', message: 'This one is broken', suggested_action: null },
      ],
    };
    const { content } = contentToVisual(uncuratedDetail(), report);
    expect(content.find((n) => n.name.includes('Local A') || n.id.includes('local-a.jar'))?.health).toBe('needs-attention');
    expect(content.find((n) => n.id.includes('local-b.jar'))?.health).toBe('healthy');
  });

  it('still attributes a finding that DOES name a registry id', () => {
    const report: HealthReport = {
      ...instanceLevelWarning,
      warnings: [
        { kind: 'bad_mod', mod_id: 'sodium', filename: null, message: 'Broken', suggested_action: null },
      ],
    };
    const { content } = contentToVisual(detail(), report);
    expect(content.find((n) => n.id.includes('sodium.jar'))?.health).toBe('needs-attention');
    expect(content.find((n) => n.id.includes('tweakeroo.jar'))?.health).toBe('healthy');
  });

  it('a failed health read still yields unknown, not healthy (SOL-2 §15 BLOCKER 1 unchanged)', () => {
    const { content } = contentToVisual(uncuratedDetail(), null, false);
    for (const node of content) expect(node.health).toBe('unknown');
  });

  it('findings render a distinct headline and body, not the same sentence twice (T6-12)', () => {
    const findings = healthToVisual(instanceLevelWarning);
    expect(findings).toHaveLength(1);
    expect(findings[0].title).not.toBe(findings[0].summary);
    expect(findings[0].summary).toMatch(/absent from mods/);
  });

  it('recommendations are categorised from their kind, not hardcoded runtime (T6-12)', () => {
    const report: HealthReport = {
      ...instanceLevelWarning,
      warnings: [],
      recommendations: [
        { kind: 'recommended_dependency', mod_id: null, source_filename: 'local-a.jar', message: "recommends 'jade'", suggested_action: null },
        { kind: 'memory', mod_id: null, source_filename: null, message: 'Use automatic memory', suggested_action: null },
      ],
    };
    const findings = healthToVisual(report);
    expect(findings[0].structuredKind).toBe('content');
    expect(findings[0].affectedIds).toHaveLength(1);
    expect(findings[1].structuredKind).toBe('runtime');
  });

  it('derives a friendly label but always keeps the exact filename (T6-13 / SOL §22.5)', () => {
    const { content } = contentToVisual(
      detail({
        manifest: {
          instance_id: 'inst-1', name: 'My World', created_from_pack: null,
          minecraft_version: '1.20.1', loader: 'fabric', loader_version: '0.15.11', is_locked: false,
          mods: [
            { filename: 'AdvancementPlaques-1.21.1-fabric-1.6.8.jar', registry_id: null, modrinth_id: null, source: 'local', version: null, sha256: 'a', installed_at: '', enabled: true, content_type: 'mod', mod_jar_id: null },
          ],
          resourcepacks: [], shaders: [], datapacks: [], worlds: [], user_preferences: {},
        },
      }),
      null,
      false,
    );
    expect(content[0].name).toBe('Advancement Plaques');
    expect(content[0].fileLabel).toBe('AdvancementPlaques-1.21.1-fabric-1.6.8.jar');
  });

  it('falls back to the filename when nothing meaningful can be derived', () => {
    expect(displayNameFromFilename('1.21.jar')).toBeNull();
    expect(displayNameFromFilename('fabric-api-0.1.jar')).toBeNull();
    expect(displayNameFromFilename('BetterF1-Fabric-1.1+1.21.7.jar')).toBe('Better F1');
    expect(displayNameFromFilename('entity_model_features_3.2.4-1.21-fabric.jar')).toBe('Entity Model Features');
  });
});
