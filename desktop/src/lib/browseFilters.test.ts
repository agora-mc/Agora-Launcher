import { describe, expect, it } from 'vitest';
import { contentTypeUnavailableReason, hiddenResultsSentence, loaderFilterApplies } from './browseFilters';

describe('browse filters', () => {
  it('applies the mod loader only to mods and modpacks', () => {
    expect(loaderFilterApplies('mod')).toBe(true);
    expect(loaderFilterApplies('pack')).toBe(true);
    expect(loaderFilterApplies(null)).toBe(true);
    expect(loaderFilterApplies('resourcepack')).toBe(false);
    expect(loaderFilterApplies('shader')).toBe(false);
    expect(loaderFilterApplies('datapack')).toBe(false);
  });

  it('explains the world catalog instead of leaving it blank', () => {
    expect(contentTypeUnavailableReason('world')).toMatch(/not available/);
    expect(contentTypeUnavailableReason('mod')).toBeNull();
  });
});

describe('hiddenResultsSentence', () => {
  it('names the version and loader that hid the matches', () => {
    expect(hiddenResultsSentence({ count: 3, more: false }, '1.21', 'fabric'))
      .toBe('No results for 1.21 Fabric. 3 results for other versions or loaders.');
  });

  it('uses the singular for one hidden match', () => {
    expect(hiddenResultsSentence({ count: 1, more: false }, '1.21', 'fabric'))
      .toBe('No results for 1.21 Fabric. 1 result for other versions or loaders.');
  });

  it('says at least when the count is a floor', () => {
    expect(hiddenResultsSentence({ count: 20, more: true }, '1.21', 'fabric'))
      .toBe('No results for 1.21 Fabric. At least 20 results for other versions or loaders.');
  });

  it('says nothing when the filter hid nothing', () => {
    expect(hiddenResultsSentence({ count: 0, more: false }, '1.21', 'fabric')).toBeNull();
  });

  it('copes with only one of the two filters being set', () => {
    expect(hiddenResultsSentence({ count: 2, more: false }, null, 'neoforge'))
      .toBe('No results for Neoforge. 2 results for other versions or loaders.');
    expect(hiddenResultsSentence({ count: 2, more: false }, '1.21', null))
      .toBe('No results for 1.21. 2 results for other versions or loaders.');
  });

  it('does not invent a scope when neither filter is set', () => {
    expect(hiddenResultsSentence({ count: 2, more: false }, null, null))
      .toBe('No results for this instance. 2 results for other versions or loaders.');
  });
});
