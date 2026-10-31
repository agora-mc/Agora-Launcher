import { describe, expect, it } from 'vitest';
import type { CheckFinding, InstanceCheck } from './api';
import {
  baseModeCopy,
  checkHeadline,
  DEPLOYMENT_OPTIONS,
  deploymentLabel,
  elapsedText,
  fallbackTradeoffs,
  formatWhen,
  frameworksMatch,
  orderFindings,
  progressText,
} from './instanceText';

function finding(overrides: Partial<CheckFinding> = {}): CheckFinding {
  return {
    kind: 'framework',
    blocks: false,
    title: 'Rule',
    detail: 'detail',
    why: 'why',
    repair: null,
    ...overrides,
  };
}

function check(overrides: Partial<InstanceCheck> = {}): InstanceCheck {
  return { instanceId: 'grounded-1', checked: true, status: 'ok', findings: [], ...overrides };
}

describe('deployment words', () => {
  it('names every mode, and automatic when no choice is recorded', () => {
    expect(deploymentLabel(null)).toBe('Automatic');
    expect(deploymentLabel('virtual')).toBe('Virtual file system');
    expect(deploymentLabel('links')).toBe('Linked files');
    expect(deploymentLabel('copies')).toBe('Copied files');
  });

  it('offers automatic first, then the three modes, each with an explanation', () => {
    expect(DEPLOYMENT_OPTIONS.map((option) => option.id)).toEqual(['auto', 'virtual', 'links', 'copies']);
    for (const option of DEPLOYMENT_OPTIONS) {
      expect(option.explain.length).toBeGreaterThan(20);
    }
  });

  it('says what running from each fallback rung costs, and that virtual costs nothing extra to say', () => {
    expect(fallbackTradeoffs('links').some((line) => line.includes('may be refused'))).toBe(true);
    expect(fallbackTradeoffs('copies').some((line) => line.includes('disk space'))).toBe(true);
    expect(fallbackTradeoffs('virtual')).toEqual([]);
  });
});

describe('base modes', () => {
  it('says a linked base shares most files with the install as hard links, not that it is free', () => {
    const linked = baseModeCopy('linked').explain;
    expect(linked).toContain('hard links');
    expect(linked).not.toContain('almost no');
    expect(baseModeCopy('copied').explain).toContain('about the size of the game');
  });
});

describe('check headlines', () => {
  it('says it is still checking before an answer arrives', () => {
    expect(checkHeadline(null)).toMatch(/Checking/);
  });

  it('says an unpinned instance has no framework check', () => {
    expect(checkHeadline(check({ checked: false }))).toMatch(/no framework check/);
  });

  it('counts only the blocking findings when the check fails', () => {
    const headline = checkHeadline(
      check({
        status: 'findings',
        findings: [finding({ blocks: true }), finding({ blocks: false }), finding({ blocks: true })],
      }),
    );
    expect(headline).toBe('2 problems stop the game from launching');
    expect(checkHeadline(check({ status: 'findings', findings: [finding({ blocks: true })] }))).toBe(
      '1 problem stops the game from launching',
    );
  });

  it('counts warnings as things to review', () => {
    expect(checkHeadline(check({ status: 'warnings', findings: [finding()] }))).toBe('1 thing to review');
    expect(checkHeadline(check({ status: 'warnings', findings: [finding(), finding()] }))).toBe('2 things to review');
  });
});

describe('frameworksMatch', () => {
  it('is unknown until a check has answered, and for an unpinned instance', () => {
    expect(frameworksMatch(null)).toBeNull();
    expect(frameworksMatch(check({ checked: false }))).toBeNull();
  });

  it('fails only on a framework finding that blocks, not on other findings', () => {
    expect(frameworksMatch(check())).toBe(true);
    expect(frameworksMatch(check({ status: 'warnings', findings: [finding({ kind: 'tool' })] }))).toBe(true);
    expect(frameworksMatch(check({ status: 'findings', findings: [finding({ kind: 'load_order', blocks: true })] }))).toBe(true);
    expect(frameworksMatch(check({ status: 'findings', findings: [finding({ blocks: true })] }))).toBe(false);
    expect(frameworksMatch(check({ status: 'warnings', findings: [finding({ blocks: false })] }))).toBe(true);
  });
});

describe('orderFindings', () => {
  it('puts the blocking findings first and keeps the rest in order', () => {
    const ordered = orderFindings([
      finding({ title: 'a', blocks: false }),
      finding({ title: 'b', blocks: true }),
      finding({ title: 'c', blocks: false }),
      finding({ title: 'd', blocks: true }),
    ]).map((entry) => entry.title);
    expect(ordered).toEqual(['b', 'd', 'a', 'c']);
  });

  it('does not change the list it was given', () => {
    const input = [finding({ title: 'a' }), finding({ title: 'b', blocks: true })];
    orderFindings(input);
    expect(input.map((entry) => entry.title)).toEqual(['a', 'b']);
  });
});

describe('formatWhen', () => {
  it('reads a saved time as UTC and never invents a time for an empty value', () => {
    expect(formatWhen(null)).toBe('Never');
    expect(formatWhen('   ')).toBe('Never');
    expect(formatWhen('2026-10-01 10:00:00')).toBe(new Date('2026-10-01T10:00:00Z').toLocaleString());
  });

  it('shows an unreadable value as it came rather than a made-up date', () => {
    expect(formatWhen('last tuesday')).toBe('last tuesday');
  });
});

describe('progress and elapsed time', () => {
  it('says starting until there is a total, and counts files with separators', () => {
    expect(progressText(0, 0)).toBe('Starting…');
    expect(progressText(1500, 12000)).toBe('1,500 of 12,000 files');
  });

  it('reads elapsed seconds as seconds, then minutes', () => {
    expect(elapsedText(0)).toBe('0 s');
    expect(elapsedText(42.9)).toBe('42 s');
    expect(elapsedText(125)).toBe('2 min 05 s');
    expect(elapsedText(-3)).toBe('0 s');
  });
});
