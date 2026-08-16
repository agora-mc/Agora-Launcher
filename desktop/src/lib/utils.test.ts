/**
 * Version sorting for pinned loader versions.
 *
 * The loader manifest ships oldest-first, so anything that defaults to
 * `versions[0]` would silently pick the oldest pinned loader. These tests pin
 * the newest-first comparator the create-instance dialogs rely on.
 */

import { describe, expect, it } from 'vitest';
import { compareVersionsDescending, sortLoaderVersionsLatestFirst } from './utils';

describe('compareVersionsDescending', () => {
  it('orders plain dotted versions newest-first', () => {
    const versions = ['0.15.11', '0.16.9', '0.16.10', '0.18.1', '0.16.10'];
    expect([...versions].sort(compareVersionsDescending)).toEqual([
      '0.18.1',
      '0.16.10',
      '0.16.10',
      '0.16.9',
      '0.15.11',
    ]);
  });

  it('compares across different segment lengths', () => {
    expect(compareVersionsDescending('1.21', '1.20.1')).toBeLessThan(0);
    expect(compareVersionsDescending('1.9', '1.21')).toBeGreaterThan(0);
    expect(compareVersionsDescending('50.1.0', '50.1.0')).toBe(0);
  });

  it('treats a release as newer than a prerelease of the same core', () => {
    expect(compareVersionsDescending('1.0.0', '1.0.0-beta.1')).toBeLessThan(0);
    expect(compareVersionsDescending('1.0.0-beta.1', '1.0.0')).toBeGreaterThan(0);
  });

  it('tolerates non-numeric segments without throwing', () => {
    expect(() => compareVersionsDescending('nightly', '0.16.9')).not.toThrow();
  });
});

describe('sortLoaderVersionsLatestFirst', () => {
  it('returns a newest-first copy without mutating the input', () => {
    const input = [
      { loader_version: '0.15.11' },
      { loader_version: '0.16.9' },
      { loader_version: '0.16.10' },
    ];
    const sorted = sortLoaderVersionsLatestFirst(input);
    expect(sorted.map((entry) => entry.loader_version)).toEqual(['0.16.10', '0.16.9', '0.15.11']);
    expect(input.map((entry) => entry.loader_version)).toEqual(['0.15.11', '0.16.9', '0.16.10']);
  });
});
