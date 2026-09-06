import { describe, expect, it } from 'vitest';
import { groupInstalledContent } from './contentTableState';
import type { InstalledContentRow } from '../../lib/tauri';

function row(overrides: Partial<InstalledContentRow> & { filename: string }): InstalledContentRow {
  return {
    key: overrides.filename,
    display_name: overrides.filename,
    version: null,
    content_type: 'mod',
    enabled: true,
    installed_at: '2026-01-01T00:00:00Z',
    source: 'modrinth',
    source_label: 'Modrinth',
    pack_managed: false,
    source_url: null,
    registry_id: null,
    modrinth_id: null,
    mod_jar_id: null,
    loader_mod_id: null,
    size_bytes: 0,
    file_present: true,
    resolved_path: null,
    author: null,
    categories: [],
    icon_url: null,
    curation_status: 'unknown',
    agora_score: null,
    modrinth_downloads: null,
    metadata_status: 'unknown',
    ...overrides,
  } as InstalledContentRow;
}

describe('groupInstalledContent', () => {
  it('returns a single unlabelled bucket when grouping is off', () => {
    const rows = [row({ filename: 'a.jar' }), row({ filename: 'b.jar' })];
    const groups = groupInstalledContent(rows, 'none');
    expect(groups).toHaveLength(1);
    expect(groups[0].rows).toEqual(rows);
  });

  it('puts pack content before user content', () => {
    const groups = groupInstalledContent(
      [row({ filename: 'mine.jar' }), row({ filename: 'packed.jar', pack_managed: true })],
      'pack',
    );
    expect(groups.map((group) => group.key)).toEqual(['pack', 'user']);
    expect(groups[0].rows.map((r) => r.filename)).toEqual(['packed.jar']);
    expect(groups[1].rows.map((r) => r.filename)).toEqual(['mine.jar']);
  });

  it('still reads sensibly for an instance with no pack content', () => {
    const groups = groupInstalledContent([row({ filename: 'mine.jar' })], 'pack');
    expect(groups.map((group) => group.label)).toEqual(['Added by you']);
  });

  // Rows carry several categories; duplicating a row across buckets would break
  // select-all and make the counts lie.
  it('places a multi-category row in exactly one bucket', () => {
    const groups = groupInstalledContent(
      [row({ filename: 'multi.jar', categories: ['Performance', 'Utility'] })],
      'category',
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].label).toBe('Performance');
    const total = groups.reduce((sum, group) => sum + group.rows.length, 0);
    expect(total).toBe(1);
  });

  it('sinks Uncategorized below named categories', () => {
    const groups = groupInstalledContent(
      [
        row({ filename: 'none.jar' }),
        row({ filename: 'zed.jar', categories: ['Zed'] }),
        row({ filename: 'alpha.jar', categories: ['Alpha'] }),
      ],
      'category',
    );
    expect(groups.map((group) => group.label)).toEqual(['Alpha', 'Zed', 'Uncategorized']);
  });

  it('groups by source label', () => {
    const groups = groupInstalledContent(
      [
        row({ filename: 'a.jar', source_label: 'Modrinth' }),
        row({ filename: 'b.jar', source_label: 'Agora Registry' }),
        row({ filename: 'c.jar', source_label: 'Modrinth' }),
      ],
      'source',
    );
    expect(groups.map((group) => group.label)).toEqual(['Agora Registry', 'Modrinth']);
    expect(groups[1].rows).toHaveLength(2);
  });

  it('never loses or duplicates a row in any mode', () => {
    const rows = [
      row({ filename: 'a.jar', pack_managed: true, categories: ['X'], source_label: 'Modrinth' }),
      row({ filename: 'b.jar', categories: [], source_label: 'Agora Registry' }),
      row({ filename: 'c.jar', pack_managed: true, categories: ['Y'], source_label: 'Modrinth' }),
    ];
    for (const mode of ['none', 'pack', 'category', 'source'] as const) {
      const flattened = groupInstalledContent(rows, mode).flatMap((group) => group.rows);
      expect(flattened).toHaveLength(rows.length);
      expect(new Set(flattened.map((r) => r.filename)).size).toBe(rows.length);
    }
  });
  describe('custom groups', () => {
    const rows = [
      row({ filename: 'sodium.jar' }),
      row({ filename: 'iris.jar' }),
      row({ filename: 'jei.jar' }),
    ];
    const assignments = { Performance: ['sodium.jar'], Visual: ['iris.jar'] };

    it('buckets by the user assignments and sinks the rest to the bottom', () => {
      const groups = groupInstalledContent(rows, 'custom', assignments);
      expect(groups.map((g) => g.label)).toEqual(['Performance', 'Visual', 'Ungrouped']);
      expect(groups[2].rows.map((r) => r.filename)).toEqual(['jei.jar']);
    });

    it('keeps every row exactly once', () => {
      const groups = groupInstalledContent(rows, 'custom', assignments);
      const flattened = groups.flatMap((g) => g.rows);
      expect(flattened).toHaveLength(rows.length);
      expect(new Set(flattened.map((r) => r.filename)).size).toBe(rows.length);
    });

    it('puts everything in Ungrouped when there are no assignments', () => {
      const groups = groupInstalledContent(rows, 'custom', {});
      expect(groups).toHaveLength(1);
      expect(groups[0].label).toBe('Ungrouped');
    });

    it('ignores assignments naming content that is not installed', () => {
      const groups = groupInstalledContent(rows, 'custom', { Gone: ['deleted.jar'] });
      expect(groups.map((g) => g.label)).toEqual(['Ungrouped']);
    });
  });
});
