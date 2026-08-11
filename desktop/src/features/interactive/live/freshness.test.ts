import { describe, expect, it } from 'vitest';
import { isExecutable, liveSource, nextRevision, requiresRefresh } from './freshness';

describe('live freshness (FIX BEFORE LIVE MODE 1)', () => {
  it('only a fresh live scene is executable', () => {
    expect(isExecutable(liveSource('r1', 'fresh'))).toBe(true);
    expect(isExecutable(liveSource('r1', 'refreshing'))).toBe(false);
    expect(isExecutable(liveSource('r1', 'stale'))).toBe(false);
    expect(isExecutable(liveSource('r1', 'unknown'))).toBe(false);
  });

  it('requiresRefresh is true unless fresh', () => {
    expect(requiresRefresh(liveSource('r1', 'fresh'))).toBe(false);
    expect(requiresRefresh(liveSource('r1', 'refreshing'))).toBe(true);
    expect(requiresRefresh(liveSource('r1', 'stale'))).toBe(true);
    expect(requiresRefresh(liveSource('r1', 'unknown'))).toBe(true);
  });

  it('nextRevision produces unique, local read-set ids', () => {
    expect(nextRevision()).not.toBe(nextRevision());
    expect(nextRevision().length).toBeGreaterThan(0);
  });
});
