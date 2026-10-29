import { describe, expect, it } from 'vitest';
import type { RunReport, SwapCosts, ToolRow } from './api';
import {
  conflictLine,
  humanBytes,
  messageOf,
  runDetailLines,
  runHeadline,
  shownPaths,
  statusLabel,
  swapCostLines,
} from './toolCopy';

const costs: SwapCosts = {
  installDir: 'D:/Games/Skyrim',
  dataFiles: 368,
  dataBytes: 5 * 1024 * 1024 * 1024 + 512 * 1024 * 1024,
  sameVolume: true,
  copyKind: 'kept',
  copyFolder: 'D:/Agora/swap',
  keptCopyExists: false,
  copyFiles: 368,
  copyBytes: 5 * 1024 * 1024 * 1024 + 512 * 1024 * 1024,
  freeBytes: 120 * 1024 * 1024 * 1024,
  steamRunning: false,
};

const report: RunReport = {
  toolId: 'nemesis',
  name: 'Nemesis',
  run: 'r1',
  capture: 'vfs',
  captureReason: null,
  promoted: true,
  current: '3',
  previous: '2',
  written: ['Data/a.hkx', 'Data/b.hkx'],
  deleted: [],
  exitCode: 0,
  cancelled: false,
  failedFolder: null,
  failure: null,
  copiedLayer: null,
  installChanges: { created: [], changed: [], deleted: [], mayBeIncomplete: false, unavailable: null },
  swap: null,
  offers: [],
  unchangedDropped: 0,
};

describe('humanBytes', () => {
  it.each([
    [0, '0 bytes'],
    [512, '512 bytes'],
    [3 * 1024 * 1024, '3 MB'],
    [5 * 1024 * 1024 * 1024 + 512 * 1024 * 1024, '5.5 GB'],
  ])('says %i bytes as %s, the way the core does', (bytes, text) => {
    expect(humanBytes(bytes)).toBe(text);
  });
});

describe('statusLabel', () => {
  it.each([
    ['current', 'Up to date'],
    ['stale', 'Out of date'],
    ['unknown', 'Rebuild to be sure'],
    ['conflict', 'Conflicts with another tool'],
    [null, 'Not built yet'],
  ] as const)('labels %s', (status, label) => {
    expect(statusLabel(status)).toBe(label);
  });
});

describe('swapCostLines', () => {
  it('says what the copy costs, that Data is renamed, and that Steam is a risk', () => {
    const lines = swapCostLines('Nemesis', { ...costs, steamRunning: true });
    expect(lines.join('\n')).toContain('needs 5.5 GB on the drive of D:/Agora/swap, and 120.0 GB is free there.');
    expect(lines.join('\n')).toContain('368 file(s)');
    expect(lines.join('\n')).toContain('renamed for the length of the run');
    expect(lines.join('\n')).toContain('Warning: Steam is running');
  });

  it('says every run copies all of Data when the copy is on another drive', () => {
    const lines = swapCostLines('Nemesis', { ...costs, sameVolume: false });
    expect(lines.join('\n')).toContain('every run copies all of Data in');
    expect(lines.join('\n')).toContain('deleted after the run');
  });

  it('says a kept copy means only changed files are copied', () => {
    const lines = swapCostLines('Nemesis', { ...costs, keptCopyExists: true });
    expect(lines.join('\n')).toContain('copies only what changed');
  });

  it('says the free space could not be read rather than showing zero', () => {
    const lines = swapCostLines('Nemesis', { ...costs, freeBytes: null });
    expect(lines.join('\n')).toContain('The free space there could not be read.');
    expect(lines.join('\n')).not.toContain('0 bytes is free');
  });
});

describe('run wording', () => {
  it('says a promoted run is the next generation, read on the next launch', () => {
    expect(runHeadline(report)).toBe('Nemesis finished, and its output is now generation 3. The game reads it on its next launch.');
  });

  it('says a discarded run changed nothing, and why', () => {
    const discarded: RunReport = { ...report, promoted: false, failure: 'exit code 3', failedFolder: 'D:/x/failed-1' };
    expect(runHeadline(discarded)).toContain('nothing changed');
    const lines = runDetailLines(discarded);
    expect(lines).toContain('Why: exit code 3');
    expect(lines).toContain('The files it wrote are kept for diagnostics in D:/x/failed-1.');
  });

  it('says a cancelled run changed nothing', () => {
    expect(runHeadline({ ...report, cancelled: true, promoted: false })).toContain('stopped before it finished');
  });

  it('names the game-folder writes Agora did not capture, and admits when they may be incomplete', () => {
    const lines = runDetailLines({
      ...report,
      installChanges: { created: ['Data/x'], changed: [], deleted: [], mayBeIncomplete: true, unavailable: null },
    });
    expect(lines.join('\n')).toContain('wrote 1 path(s) in the game folder');
    expect(lines.join('\n')).toContain('Changes may have been missed');
  });

  it('says why a run ran another way than the default', () => {
    const lines = runDetailLines({ ...report, capture: 'links', captureReason: 'the file layer could not start' });
    expect(lines).toContain('Captured by link capture.');
    expect(lines).toContain('It runs this way because: the file layer could not start');
  });
});

describe('shownPaths', () => {
  it('shows twenty and counts the rest', () => {
    const paths = Array.from({ length: 23 }, (_, i) => `f${i}`);
    const { shown, more } = shownPaths(paths);
    expect(shown).toHaveLength(20);
    expect(more).toBe(3);
  });

  it.each([[[]], [['a']]])('counts nothing extra for a short list (%j)', (paths) => {
    expect(shownPaths(paths).more).toBe(0);
  });
});

describe('conflictLine', () => {
  const nemesis: ToolRow = { toolId: 'nemesis', name: 'Nemesis', declared: true, current: '1', previous: null, status: 'current', conflicts: true };
  const pandora: ToolRow = { ...nemesis, toolId: 'pandora', name: 'Pandora', conflicts: true };
  const bodyslide: ToolRow = { ...nemesis, toolId: 'bodyslide', name: 'BodySlide', conflicts: false };

  it('names the other conflicting tool, not the ones that do not conflict', () => {
    const text = conflictLine(nemesis, [nemesis, pandora, bodyslide]);
    expect(text).toBe('Nemesis and Pandora both have output here, and both write the same behaviour files. Remove the one you do not want to use.');
  });

  it('does not name the tool itself when it is the only one listed', () => {
    expect(conflictLine(nemesis, [nemesis])).toContain('Nemesis and another tool');
  });
});

describe('messageOf', () => {
  it('shows a GamesError message, an Error message, or a plain line for anything else', () => {
    expect(messageOf({ code: 'core_failed', message: 'The game is running.' })).toBe('The game is running.');
    expect(messageOf(new Error('boom'))).toBe('boom');
    expect(messageOf(undefined)).toBe('Something went wrong. Try again.');
    expect(messageOf({ message: 'no code' })).toBe('Something went wrong. Try again.');
  });
});
