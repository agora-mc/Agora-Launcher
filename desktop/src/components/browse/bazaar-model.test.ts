/**
 * Bazaar model tests — the taste model must stay legible and deterministic,
 * the recommender must reorder visibly, and "Surprise me" must never return
 * something owned.
 */

import { describe, expect, it, vi } from 'vitest';
import {
  categoryTags,
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
  it('maps real Modrinth categories onto the taste axes transparently', () => {
    expect(vibesFor(item({ categories: ['adventure', 'worldgen'] }))).toEqual(['adventure', 'worldgen']);
    expect(vibesFor(item({ categories: ['technology', 'optimization'] }))).toEqual(['technology', 'optimization']);
    expect(vibesFor(item({ categories: ['decoration'] }))).toEqual(['decoration']);
    expect(vibesFor(item({ categories: ['magic', 'mobs'] }))).toEqual(['magic', 'mobs']);
    // every item has at least one axis, even without categories
    expect(vibesFor(item({ categories: [] }))).toEqual(['utility']);
  });

  it("❤️ adds to the item's categories, 💔 subtracts, tapping again cancels", () => {
    const st = vote(initialBazaarState(), item({ categories: ['adventure'] }), 1);
    expect(st.taste.adventure).toBe(1);
    const cancelled = vote(st, item({ categories: ['adventure'] }), 1);
    expect(cancelled.taste.adventure).toBe(0);
    const down = vote(cancelled, item({ categories: ['adventure'] }), -1);
    expect(down.taste.adventure).toBe(-1);
  });

  it('owned items sort to the back (scoreOf −99)', () => {
    const a = item({ id: 'a', name: 'Alpha', categories: ['adventure'] });
    const b = item({ id: 'b', name: 'Beta', categories: ['magic'] });
    const st = vote(initialBazaarState(), a, 1); // a scores +1 on adventure
    expect(scoreOf(st, a)).toBe(1);
    expect(scoreOf(st, b)).toBe(0);
    const owned = { ...st, owned: { a: true } };
    expect(scoreOf(owned, a)).toBe(-98);
    const sorted = sortedShelf(owned, [a, b]);
    expect(sorted[0].id).toBe('b');
    expect(sorted[1].id).toBe('a');
  });

  it('curated items get the small curated nudge in score', () => {
    const a = item({ id: 'a', name: 'Alpha', categories: ['adventure'], curated: true });
    const b = item({ id: 'b', name: 'Beta', categories: ['adventure'] });
    const st = initialBazaarState();
    expect(scoreOf(st, a)).toBeCloseTo(0.6);
    expect(scoreOf(st, b)).toBe(0);
  });

  it('topVibe and bar widths stay legible', () => {
    let st = initialBazaarState();
    st = vote(st, item({ id: 'a', categories: ['adventure'] }), 1);
    st = vote(st, item({ id: 'b', categories: ['technology'] }), 1);
    st = vote(st, item({ id: 'c', categories: ['magic'] }), 1);
    expect(topVibe(st)).toBe('adventure');
    const widths = vibeBarWidths(st);
    expect(widths.adventure).toBe(100);
    expect(widths.technology).toBe(100);
    expect(widths.magic).toBe(100);
    expect(widths.utility).toBe(0);
  });

  it('categoryTags formats real modrinth slugs and dedupes', () => {
    expect(categoryTags(item({ categories: ['game-mechanics', 'utility', 'utility'] }))).toEqual(['Game Mechanics', 'Utility']);
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

describe('sortedShelf preserves the backend ranking', () => {
  const item = (id: string, name: string): BazaarItem => ({
    id, name, iconUrl: null, description: null, contentType: 'mod',
    author: null, categories: [], supportedVersions: [],
  });
  // Deliberately reverse-alphabetical, as a net-score ordering usually is.
  const backendOrder = [item('1', 'Zebra'), item('2', 'Mango'), item('3', 'Apple')];

  it('keeps the given order when taste is neutral (never alphabetises)', () => {
    const state = initialBazaarState();
    const out = sortedShelf(state, backendOrder).map((i) => i.name);
    expect(out).toEqual(['Zebra', 'Mango', 'Apple']);
  });

  it('still lets taste outrank the backend order', () => {
    const liked = { ...item('4', 'Aardvark'), categories: ['adventure'] };
    const state = initialBazaarState();
    state.taste.adventure = 5;
    const out = sortedShelf(state, [...backendOrder, liked]).map((i) => i.name);
    expect(out[0]).toBe('Aardvark');
    // and the rest keep backend order behind it
    expect(out.slice(1)).toEqual(['Zebra', 'Mango', 'Apple']);
  });
});
