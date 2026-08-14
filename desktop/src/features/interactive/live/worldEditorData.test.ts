/**
 * Rarity and dependency relationships in the world editor.
 *
 * These two features share one input. Before the dependency-graph read existed,
 * `scene.relationships` was built ONLY from health blockers, so a healthy
 * instance produced no edges at all: nothing to draw curves between, and
 * `requiredBy` stuck at 0 so every single item came out `common`. Both looked
 * "implemented but dead". These tests pin the derivation so that cannot recur
 * silently.
 */

import { describe, expect, it } from 'vitest';
import { contentToVisual } from './readAdapters';
import { buildEditorData } from './worldEditorData';
import type { DependencyEdge, InstalledMod, InstanceDetail } from '@/lib/tauri';

function mod(filename: string): InstalledMod {
  return {
    filename,
    registry_id: filename.replace('.jar', ''),
    modrinth_id: null,
    source: 'registry',
    version: '1.0',
    sha256: 'x',
    installed_at: '',
    enabled: true,
    content_type: 'mod',
    mod_jar_id: null,
  };
}

function detailWith(filenames: string[]): InstanceDetail {
  return {
    row: {
      instance_id: 'i1',
      name: 'Test',
      minecraft_version: '1.20.1',
      loader: 'fabric',
      loader_version: '0.15',
      is_locked: false,
    },
    manifest: {
      instance_id: 'i1',
      name: 'Test',
      created_from_pack: null,
      minecraft_version: '1.20.1',
      loader: 'fabric',
      loader_version: '0.15',
      is_locked: false,
      mods: filenames.map(mod),
      resourcepacks: [],
      shaders: [],
      datapacks: [],
      worlds: [],
      user_preferences: {},
    },
    snapshot_readiness: 'ready',
    snapshot_error: null,
  } as unknown as InstanceDetail;
}

/** `needed` mods all depend on `lib.jar`. */
function edgesOnto(target: string, dependents: string[]): DependencyEdge[] {
  return dependents.map((from) => ({
    from_filename: from,
    to_filename: target,
    requirement: 'required' as const,
  }));
}

function build(filenames: string[], edges: DependencyEdge[]) {
  const { content, relationships } = contentToVisual(detailWith(filenames), null, true, edges);
  return buildEditorData(
    { source: { revision: 1, freshness: 'fresh' }, content, relationships, findings: [], proposals: [] } as never,
    false,
  );
}

describe('world editor: dependency relationships', () => {
  it('produces a drawable edge for every installed-to-installed dependency', () => {
    const data = build(['caves.jar', 'lib.jar'], edgesOnto('lib.jar', ['caves.jar']));
    // Single-word filenames derive no friendlier label, so the name IS the filename.
    const caves = data.items.find((i) => i.name === 'caves.jar')!;
    const lib = data.items.find((i) => i.name === 'lib.jar')!;
    expect(caves.needs).toEqual([lib.name]);
    expect(lib.neededBy).toEqual([caves.name]);
  });

  it('a healthy instance still has relationships (the regression that killed the curves)', () => {
    // No health report at all — previously this meant zero relationships.
    const data = build(['a.jar', 'b.jar'], edgesOnto('b.jar', ['a.jar']));
    expect(data.items.some((i) => i.needs.length > 0)).toBe(true);
  });
});

describe('world editor: rarity is derived from how many mods depend on an item', () => {
  const dependents = (n: number) => Array.from({ length: n }, (_, i) => `dep${i}.jar`);

  function rarityOfLibWith(n: number) {
    const files = ['lib.jar', ...dependents(n)];
    const data = build(files, edgesOnto('lib.jar', dependents(n)));
    return data.items.find((i) => i.name.toLowerCase().includes('lib'))!.rarity;
  }

  it('nothing depends on it -> common', () => {
    expect(rarityOfLibWith(0)).toBe('common');
  });

  it('a single dependent -> rare', () => {
    expect(rarityOfLibWith(1)).toBe('rare');
  });

  it('four dependents -> epic', () => {
    expect(rarityOfLibWith(4)).toBe('epic');
  });

  it('ten dependents -> legendary', () => {
    expect(rarityOfLibWith(10)).toBe('legendary');
  });

  it('rarity rises with dependency count and never falls', () => {
    const order = { common: 0, rare: 1, epic: 2, legendary: 3 };
    let last = -1;
    for (const n of [0, 1, 3, 4, 9, 10, 14]) {
      const r = order[rarityOfLibWith(n)];
      expect(r).toBeGreaterThanOrEqual(last);
      last = r;
    }
  });
});
