/**
 * Bazaar model tests — the taste model must stay legible and deterministic,
 * the recommender must reorder visibly, and "Surprise me" must never return
 * something owned (V5-PORT-PLAN §11 rules).
 */

import { describe, expect, it, vi } from 'vitest';
import {
  crank,
  fitFor,
  hashOf,
  initialBazaarState,
  isOwned,
  scoreOf,
  sortedShelf,
  stageItem,
  topVibe,
  vibeBarWidths,
  vibesFor,
  vote,
  type BazaarItem,
} from './bazaar-model';

function item(over: Partial<BazaarItem> = {}): BazaarItem {
  return {
    id: 'i-1',
    name: 'Test Mod',
    iconUrl: null,
    description: null,
    contentType: 'mod',
    author: null,
    categories: ['adventure'],
    supportedVersions: ['1.20.1'],
    ...over,
  };
}

describe('bazaar taste model', () => {
  it('maps categories onto the five axes transparently', () => {
    expect(vibesFor(item({ contentType: 'shader' }))).toContain('pretty');
    expect(vibesFor(item({ categories: ['adventure', 'biome'] }))).toContain('wild');
    expect(vibesFor(item({ categories: ['technology'] }))).toContain('tricky');
    expect(vibesFor(item({ categories: ['building', 'furniture'] }))).toContain('cosy');
    expect(vibesFor(item({ categories: ['funny'] }))).toContain('silly');
    // every item has at least one axis
    expect(vibesFor(item({ categories: [] })).length).toBeGreaterThan(0);
  });

  it('👍 adds to the item vibes, 👎 subtracts, tapping again cancels', () => {
    const st = vote(initialBazaarState(), item({ categories: ['adventure'] }), 1);
    expect(st.taste.wild).toBe(1);
    const cancelled = vote(st, item({ categories: ['adventure'] }), 1);
    expect(cancelled.taste.wild).toBe(0);
    const down = vote(cancelled, item({ categories: ['adventure'] }), -1);
    expect(down.taste.wild).toBe(-1);
  });

  it('owned items sort to the back (scoreOf −99)', () => {
    const a = item({ id: 'a', name: 'Alpha', categories: ['adventure'] });
    const b = item({ id: 'b', name: 'Beta', categories: ['funny'] });
    const st = vote(initialBazaarState(), a, 1); // a scores +1 on wild
    expect(scoreOf(st, a)).toBe(1);
    expect(scoreOf(st, b)).toBe(0);
    const owned = { ...st, owned: { a: true } };
    expect(scoreOf(owned, a)).toBe(-98);
    const sorted = sortedShelf(owned, [a, b]);
    expect(sorted[0].id).toBe('b');
    expect(sorted[1].id).toBe('a');
  });

  it('topVibe and bar widths stay legible', () => {
    let st = initialBazaarState();
    st = vote(st, item({ id: 'a', categories: ['adventure'] }), 1);
    st = vote(st, item({ id: 'b', categories: ['technology'] }), 1);
    st = vote(st, item({ id: 'c', categories: ['funny'] }), 1);
    expect(topVibe(st)).toBe('wild');
    const widths = vibeBarWidths(st);
    expect(widths.wild).toBe(100);
    expect(widths.tricky).toBe(100);
    expect(widths.silly).toBe(100);
    expect(widths.cosy).toBe(0);
  });

  it('hashOf is deterministic', () => {
    expect(hashOf('Cobblemon')).toBe(hashOf('Cobblemon'));
    expect(hashOf('Cobblemon')).not.toBe(hashOf('Sodium'));
  });
});

describe('bazaar surprise-me machine', () => {
  it('never returns something owned', () => {
    const a = item({ id: 'a', name: 'A' });
    const b = item({ id: 'b', name: 'B' });
    const st = { ...initialBazaarState(), owned: { a: true } };
    vi.spyOn(Math, 'random').mockReturnValue(0.999); // would pick the last candidate if allowed
    try {
      const pick = crank(st, [a, b]);
      expect(pick).not.toBeNull();
      expect(pick!.id).not.toBe('a');
    } finally {
      vi.restoreAllMocks();
    }
  });

  it('returns null when everything is owned', () => {
    const st = { ...initialBazaarState(), owned: { a: true, b: true } };
    expect(crank(st, [item({ id: 'a', name: 'A' }), item({ id: 'b', name: 'B' })])).toBeNull();
  });

  it('weights by taste (a liked item is more likely)', () => {
    const a = item({ id: 'a', name: 'A', categories: ['adventure'] });
    const b = item({ id: 'b', name: 'B', categories: ['adventure'] });
    const st = vote(initialBazaarState(), a, 1); // a scores +1
    // Mock random to a small value: with weights max(0.35, 1+score), a (score 1)
    // gets weight 2, b (score 0) gets 1. A random of 0.4 of total 3 lands in a.
    const rand = vi.spyOn(Math, 'random').mockReturnValue(0.4);
    try {
      expect(crank(st, [a, b])!.id).toBe('a');
    } finally {
      rand.mockRestore();
    }
  });
});

describe('bazaar fit line', () => {
  it('reuses the version-compatibility question (same as the pre-flight)', () => {
    expect(fitFor(item({ supportedVersions: ['1.20.1', '1.21'] }), '1.20.1')).toBe(true);
    expect(fitFor(item({ supportedVersions: ['1.21'] }), '1.20.1')).toBe(false);
    // no instance context -> neutral, not a claim
    expect(fitFor(item({ supportedVersions: ['1.20.1'] }), null)).toBeNull();
    // unknown versions -> neutral
    expect(fitFor(item({ supportedVersions: [] }), '1.20.1')).toBeNull();
  });

  it('staging is reversible and tracked', () => {
    const st = stageItem(initialBazaarState(), item({ id: 'a' }));
    expect(isOwned(st, 'a')).toBe(true);
    const un = { ...st };
    delete un.staged.a;
    expect(isOwned(un, 'a')).toBe(false);
  });
});
