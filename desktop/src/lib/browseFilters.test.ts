import { describe, expect, it } from 'vitest';
import { contentTypeUnavailableReason, loaderFilterApplies } from './browseFilters';

describe('browse filters', () => {
  it('applies the mod loader only to mods and modpacks, never to All types', () => {
    expect(loaderFilterApplies('mod')).toBe(true);
    expect(loaderFilterApplies('pack')).toBe(true);
    // All types must not hide resource packs, shaders and data packs behind the loader.
    expect(loaderFilterApplies(null)).toBe(false);
    expect(loaderFilterApplies('resourcepack')).toBe(false);
    expect(loaderFilterApplies('shader')).toBe(false);
    expect(loaderFilterApplies('datapack')).toBe(false);
  });

  it('explains the world catalog instead of leaving it blank', () => {
    expect(contentTypeUnavailableReason('world')).toMatch(/not available/);
    expect(contentTypeUnavailableReason('mod')).toBeNull();
  });
});
