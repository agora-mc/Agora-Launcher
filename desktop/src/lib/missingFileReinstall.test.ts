import { describe, expect, it } from 'vitest';
import { reinstallIntent, reinstallSourceFor, searchQueryForFile } from './missingFileReinstall';
import type { InstalledMod } from './tauri';

function entry(overrides: Partial<InstalledMod>): InstalledMod {
  return {
    filename: 'iris-1.8.0.jar',
    registry_id: null,
    modrinth_id: null,
    source: 'modrinth',
    version: '1.8.0',
    sha256: 'a'.repeat(64),
    installed_at: '2026-01-01T00:00:00Z',
    enabled: true,
    content_type: 'mod',
    ...overrides,
  };
}

describe('reinstallSourceFor', () => {
  it('prefers a curated registry id over a Modrinth id, as Browse does', () => {
    expect(reinstallSourceFor(entry({ registry_id: 'iris', modrinth_id: 'YL57xq9U' })))
      .toEqual({ kind: 'curated', itemId: 'iris', version: '1.8.0' });
  });

  it('falls back to the Modrinth project when there is no registry id', () => {
    expect(reinstallSourceFor(entry({ modrinth_id: 'YL57xq9U' })))
      .toEqual({ kind: 'modrinth', projectId: 'YL57xq9U', version: '1.8.0' });
  });

  it('names no source for an imported mod with neither id', () => {
    expect(reinstallSourceFor(entry({}))).toBeNull();
  });

  it('treats blank ids as absent', () => {
    expect(reinstallSourceFor(entry({ registry_id: '   ', modrinth_id: '' }))).toBeNull();
    expect(reinstallSourceFor(entry({ registry_id: ' iris ' })))
      .toEqual({ kind: 'curated', itemId: 'iris', version: '1.8.0' });
  });

  it('names no source without a recorded version to reinstall', () => {
    expect(reinstallSourceFor(entry({ registry_id: 'iris', version: null }))).toBeNull();
    expect(reinstallSourceFor(entry({ registry_id: 'iris', version: '  ' }))).toBeNull();
  });

  it('only offers reinstall for mod jars', () => {
    expect(reinstallSourceFor(entry({ registry_id: 'iris', content_type: 'resourcepack' }))).toBeNull();
  });
});

describe('reinstallIntent', () => {
  it('builds the same install intent Browse sends, with replace turned off', () => {
    expect(reinstallIntent('inst-1', 'modrinth', 'YL57xq9U', 'ver-id-9')).toEqual({
      action: { type: 'install', sourceType: 'modrinth', itemId: 'YL57xq9U', candidateVersion: 'ver-id-9' },
      targetInstance: 'inst-1',
      optionalDeps: { type: 'prompt' },
      requestedBy: 'interactive',
      overrides: { allowReplace: false, skipHealthScan: false, forceConflictResolution: {} },
    });
  });
});

describe('searchQueryForFile', () => {
  it('drops the jar extension, including a disabled suffix', () => {
    expect(searchQueryForFile('sodium-fabric-0.6.0.jar')).toBe('sodium-fabric-0.6.0');
    expect(searchQueryForFile('Iris.JAR.disabled')).toBe('Iris');
  });

  it('keeps a name that is not a jar, and never returns an empty query', () => {
    expect(searchQueryForFile('notes.txt')).toBe('notes.txt');
    expect(searchQueryForFile('.jar')).toBe('.jar');
    expect(searchQueryForFile('  ')).toBe('  ');
  });
});
