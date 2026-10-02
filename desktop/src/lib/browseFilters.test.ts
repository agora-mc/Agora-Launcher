import { describe, expect, it } from 'vitest';
import { contentTypeUnavailableReason, loaderFilterApplies } from './browseFilters';

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
