import { describe, expect, it } from 'vitest';
import { contentTypeLabel, installContentType, projectFormats, versionFormat } from './modrinthFormats';

describe('modrinth formats', () => {
  it("reads the format from a version's loader tags", () => {
    expect(versionFormat(['fabric', 'quilt'])).toBe('mod');
    expect(versionFormat(['datapack'])).toBe('datapack');
    expect(versionFormat(['minecraft'])).toBe('mod');
  });

  it('lists every format a mixed project ships, mods first', () => {
    // VeinMiner: Fabric/Quilt jars plus a data pack.
    const versions = [{ loaders: ['datapack'] }, { loaders: ['fabric', 'quilt'] }];
    expect(projectFormats(versions)).toEqual(['mod', 'datapack']);
    expect(projectFormats([{ loaders: ['datapack'] }])).toEqual(['datapack']);
  });

  it("installs a mod project's data-pack version as a data pack only", () => {
    expect(installContentType('mod', ['datapack'])).toBe('datapack');
    expect(installContentType('mod', ['fabric'])).toBe('mod');
    expect(installContentType('resourcepack', ['minecraft'])).toBe('resourcepack');
    expect(installContentType('shader', ['iris'])).toBe('shader');
  });

  it('names content types for people', () => {
    expect(contentTypeLabel('datapack')).toBe('data pack');
    expect(contentTypeLabel('resourcepack')).toBe('resource pack');
    expect(contentTypeLabel('mod')).toBe('mod');
  });
});
