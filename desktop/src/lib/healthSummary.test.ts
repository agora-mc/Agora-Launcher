import { describe, expect, it } from 'vitest';
import {
  describeRequirement,
  healthCounts,
  healthHeadline,
  healthLaunchNote,
} from './healthSummary';

describe('health summary', () => {
  it('counts the same way everywhere', () => {
    const counts = healthCounts({ blockers: [1, 2, 3, 4, 5, 6], warnings: [1] });
    expect(counts).toEqual({ problems: 6, warnings: 1 });
    expect(healthHeadline(counts)).toBe('6 mod problems to fix · 1 warning to review');
    expect(healthLaunchNote(counts)).toMatch(/Launch is blocked/);
    expect(healthLaunchNote({ problems: 0, warnings: 2 })).not.toMatch(/blocked/);
  });

  it('explains a requirement on the other loader plainly', () => {
    const text = describeRequirement(
      { targetId: 'neoforge', versionRanges: ['[20.2,)'], verdict: 'unsatisfied', modIds: ['sodium-extra'] },
      'forge',
    );
    expect(text).toBe('sodium-extra is made for NeoForge, not Forge; it may not work in this instance.');
  });

  it('keeps ordinary unmet and met requirements readable', () => {
    const base = { targetId: 'forge', versionRanges: ['[47,)'], modIds: ['a'] };
    expect(describeRequirement({ ...base, verdict: 'satisfied' }, 'forge')).toBe('a: Forge [47,) is met.');
    expect(describeRequirement({ ...base, verdict: 'unsatisfied' }, 'forge')).toMatch(/needs Forge \[47,\)/);
  });
});
