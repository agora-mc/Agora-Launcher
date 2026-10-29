import { describe, expect, it } from 'vitest';
import {
  importHasIssues,
  importHeadline,
  mergeRetry,
  needsActionOutcomes,
} from './packImport';
import type { PackEntryOutcome, PackImportReport } from './tauri';

const entry = (filename: string) => ({
  filename, version: '1', registry_id: null, modrinth_id: 'x', sha256: null, source: 'modrinth_raw',
});
const outcome = (name: string, status: PackEntryOutcome['status'], extra: Partial<PackEntryOutcome> = {}): PackEntryOutcome => ({
  name, status, verified_against_pack: status === 'installed', reason: null, action: null, entry: entry(name), ...extra,
});
const report = (outcomes: PackEntryOutcome[]): PackImportReport => ({
  instance_id: 'pack', instance_name: 'Pack', total: outcomes.length,
  installed: outcomes.filter((o) => o.status === 'installed').length, outcomes, warnings: [],
});

describe('pack import report helpers', () => {
  it('says how many mods installed out of how many', () => {
    const partial = report([outcome('a.jar', 'installed'), outcome('b.jar', 'needs_action', { action: 'enable_modrinth' })]);
    expect(importHeadline(partial)).toBe('1 of 2 mods installed.');
    expect(importHasIssues(partial)).toBe(true);
  });

  it('is clean only when everything installed and was checked against the pack', () => {
    expect(importHasIssues(report([outcome('a.jar', 'installed')]))).toBe(false);
    const unchecked = report([outcome('a.jar', 'installed', { verified_against_pack: false, reason: 'not checked' })]);
    expect(importHasIssues(unchecked)).toBe(true);
  });

  it('merging a retry replaces only the needs-action outcomes', () => {
    const before = report([
      outcome('a.jar', 'installed'),
      outcome('b.jar', 'failed', { reason: 'bad' }),
      outcome('c.jar', 'needs_action', { action: 'enable_modrinth' }),
    ]);
    const merged = mergeRetry(before, report([outcome('c.jar', 'installed')]));
    expect(merged.outcomes.map((o) => o.status)).toEqual(['installed', 'failed', 'installed']);
    expect(merged.installed).toBe(2);
    expect(merged.total).toBe(3);
    expect(needsActionOutcomes(merged)).toHaveLength(0);
  });
});
