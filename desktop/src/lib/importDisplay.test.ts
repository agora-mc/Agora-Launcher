import { describe, expect, it } from 'vitest';
import {
  contentCountsLabel,
  copiedMemoryNote,
  formatMemoryMb,
  importProgressPercent,
  importSourceLabel,
  outcomeDisplayName,
  readCopiedMemory,
} from './importDisplay';

describe('formatMemoryMb', () => {
  it('uses the same words as the editor label for copied and slider values', () => {
    expect(formatMemoryMb(12064)).toBe('11.8 GB');
    expect(formatMemoryMb(12288)).toBe('12 GB');
    expect(formatMemoryMb(4096)).toBe('4 GB');
    expect(formatMemoryMb(2560)).toBe('2.5 GB');
    expect(formatMemoryMb(512)).toBe('512 MB');
  });
});

describe('importProgressPercent', () => {
  it('uses bytes when a byte total is known', () => {
    expect(importProgressPercent({ progress: 0, bytesDownloaded: 600, bytesTotal: 1200 })).toBe(50);
  });

  it('falls back to the fraction when there is no byte total', () => {
    expect(importProgressPercent({ progress: 0.25, bytesDownloaded: null, bytesTotal: null })).toBe(25);
    expect(importProgressPercent({ progress: 1, bytesDownloaded: 0, bytesTotal: 0 })).toBe(100);
  });

  it('is zero with no event yet, and never NaN or out of range', () => {
    expect(importProgressPercent(null)).toBe(0);
    expect(importProgressPercent({ progress: null, bytesDownloaded: null, bytesTotal: null })).toBe(0);
    expect(importProgressPercent({ progress: Number.NaN, bytesDownloaded: null, bytesTotal: null })).toBe(0);
    expect(importProgressPercent({ progress: -3, bytesDownloaded: null, bytesTotal: null })).toBe(0);
    expect(importProgressPercent({ progress: 0, bytesDownloaded: 5000, bytesTotal: 1000 })).toBe(100);
  });
});

describe('importSourceLabel', () => {
  it('names the launcher for both the snake_case kind and the source label', () => {
    expect(importSourceLabel('curse_forge')).toBe('CurseForge');
    expect(importSourceLabel('curseforge')).toBe('CurseForge');
    expect(importSourceLabel('modrinth')).toBe('Modrinth');
    expect(importSourceLabel('prism')).toBe('Prism Launcher');
  });

  it('returns null for unknown, empty, or hostile sources', () => {
    expect(importSourceLabel('')).toBeNull();
    expect(importSourceLabel('   ')).toBeNull();
    expect(importSourceLabel(null)).toBeNull();
    expect(importSourceLabel('CURSEFORGE ')).toBe('CurseForge');
    expect(importSourceLabel('<img src=x>')).toBeNull();
  });
});

describe('copiedMemoryNote', () => {
  const copied = { launcher: 'curseforge', mb: 12064 };

  it('says where the memory came from while the copied value is in use', () => {
    expect(copiedMemoryNote(copied, 12064, 'manual')).toBe('Copied from CurseForge: 11.8 GB');
  });

  it('shows nothing once automatic memory is on, or the value was changed', () => {
    expect(copiedMemoryNote(copied, 12064, 'auto')).toBeNull();
    expect(copiedMemoryNote(copied, 12288, 'manual')).toBeNull();
    expect(copiedMemoryNote(null, 12064, 'manual')).toBeNull();
  });

  it('rejects a non-numeric copied value rather than printing NaN', () => {
    expect(copiedMemoryNote({ launcher: 'curseforge', mb: Number.NaN }, Number.NaN, 'manual')).toBeNull();
  });
});

describe('readCopiedMemory', () => {
  it('reads a well-formed record and ignores anything else', () => {
    expect(readCopiedMemory({ memory_copied: { launcher: 'curseforge', mb: 12064 } })).toEqual({
      launcher: 'curseforge',
      mb: 12064,
    });
    expect(readCopiedMemory({})).toBeNull();
    expect(readCopiedMemory(null)).toBeNull();
    expect(readCopiedMemory('memory_copied')).toBeNull();
    expect(readCopiedMemory({ memory_copied: { launcher: 'curseforge', mb: '12064' } })).toBeNull();
    expect(readCopiedMemory({ memory_copied: { launcher: 7, mb: 12064 } })).toBeNull();
  });
});

describe('contentCountsLabel', () => {
  it('counts each kind with the right plural', () => {
    expect(
      contentCountsLabel({ mod_count: 12, resourcepack_count: 1, shaderpack_count: 2, world_count: 0 }),
    ).toBe('12 mods · 1 resource pack · 2 shader packs · 0 worlds');
    expect(
      contentCountsLabel({ mod_count: 1, resourcepack_count: 0, shaderpack_count: 1, world_count: 1 }),
    ).toBe('1 mod · 0 resource packs · 1 shader pack · 1 world');
  });

  it('says a count is unknown instead of printing a number', () => {
    expect(
      contentCountsLabel({ mod_count: null, resourcepack_count: 3, shaderpack_count: undefined, world_count: 2 }),
    ).toBe('mods count unknown · 3 resource packs · shader packs count unknown · 2 worlds');
  });
});

describe('outcomeDisplayName', () => {
  const items = [
    { destination_id: 'better-mc--forge----bmc4', destination_name: 'Better MC [FORGE] - BMC4' },
    { destination_id: 'blank', destination_name: '   ' },
  ];

  it('shows the planned display name instead of the id', () => {
    expect(outcomeDisplayName('better-mc--forge----bmc4', items)).toBe('Better MC [FORGE] - BMC4');
  });

  it('falls back to the id when no name is known or the name is blank', () => {
    expect(outcomeDisplayName('unknown-id', items)).toBe('unknown-id');
    expect(outcomeDisplayName('blank', items)).toBe('blank');
  });
});
