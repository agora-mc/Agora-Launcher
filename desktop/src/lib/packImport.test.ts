import { describe, expect, it } from 'vitest';
import {
  confirmHashOutcomes,
  importHasIssues,
  importHeadline,
  mergeRetry,
  needsActionOutcomes,
  packOnlyOutcomes,
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

  it('merging a partial retry replaces only the outcomes that were retried', () => {
    const before = report([
      outcome('wait.jar', 'needs_action', { action: 'enable_modrinth' }),
      outcome('differs.jar', 'needs_action', { action: 'confirm_hash', reason: 'does not match the hash in the modpack' }),
    ]);
    expect(confirmHashOutcomes(before).map((o) => o.name)).toEqual(['differs.jar']);
    const merged = mergeRetry(
      before,
      report([outcome('differs.jar', 'installed', { verified_against_pack: false })]),
      (o) => o.status === 'needs_action' && o.action === 'confirm_hash',
    );
    expect(merged.outcomes.map((o) => [o.name, o.status])).toEqual([
      ['wait.jar', 'needs_action'],
      ['differs.jar', 'installed'],
    ]);
    expect(merged.installed).toBe(1);
  });

  it('a file only the modpack vouched for is named, and is not a clean import', () => {
    const only = outcome('old.jar', 'installed', {
      reason: "Matches the hash in the modpack. The source published no checksum, so the modpack's hash is the only check on this file.",
    });
    expect(packOnlyOutcomes(report([only, outcome('b.jar', 'installed')]))).toEqual([only]);
    expect(importHasIssues(report([only]))).toBe(true);
    // The retry's note on a file that was already there is not that.
    const there = outcome('c.jar', 'installed', { reason: 'Already installed (verified against the pack).' });
    expect(packOnlyOutcomes(report([there]))).toEqual([]);
    expect(importHasIssues(report([there]))).toBe(false);
  });
});
