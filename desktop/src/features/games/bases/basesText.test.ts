import { describe, expect, it } from 'vitest';
import type { BaseFileProblem, BaseSummary, BuildProgressEvent, VerifyReply } from './api';
import {
  canRemove,
  DAMAGED_EXPLANATION,
  DAMAGED_REMOVE_BODY,
  describeProblem,
  describeUsers,
  diskText,
  gameWritesNote,
  linkedWarning,
  progressPercent,
  progressText,
  verifySummary,
} from './basesText';

const base: BaseSummary = {
  baseId: 'skyrim-se_steam_1.6.1170_nobuild',
  gameId: 'skyrim-se',
  store: 'steam',
  version: '1.6.1170',
  build: null,
  mode: 'linked',
  location: 'D:/AgoraBases/skyrim-se_steam_1.6.1170_nobuild',
  present: true,
  fileCount: 117,
  linkedFileCount: 40,
  linkedBytes: 16 * 1024 ** 3,
  copiedBytes: 200 * 1024 ** 2,
  skippedCount: 0,
  createdUnixMs: 1700000000000,
  usedBy: [],
};

const problem = (over: Partial<BaseFileProblem>): BaseFileProblem => ({
  path: 'Data/a.bsa',
  kind: 'missing',
  expectedBytes: null,
  actualBytes: null,
  linkedToStore: false,
  ...over,
});

describe('canRemove and describeUsers', () => {
  it('allows removal only when no instance uses the base', () => {
    expect(canRemove(base)).toBe(true);
    expect(canRemove({ ...base, usedBy: [{ instanceId: 'a', name: 'Nordic Run' }] })).toBe(false);
  });

  it('names every instance that uses the base, in the order given', () => {
    const text = describeUsers([
      { instanceId: 'a', name: 'Nordic Run' },
      { instanceId: 'b', name: 'Vanilla Plus' },
    ]);
    expect(text).toBe('Used by Nordic Run, Vanilla Plus. Remove or move those instances before removing this base.');
  });

  it('says a base with no users can be removed', () => {
    expect(describeUsers([])).toContain('can be removed');
  });

  it('keeps an instance whose name is blank in the list, named by its id', () => {
    // A user with an empty name is still a user: removal must stay refused, and the list shows it.
    expect(canRemove({ ...base, usedBy: [{ instanceId: 'ghost', name: '' }] })).toBe(false);
    expect(describeUsers([{ instanceId: 'ghost', name: '  ' }])).toBe('Used by ghost. Remove or move those instances before removing this base.');
  });
});

describe('diskText', () => {
  it('says a copied base holds its own bytes', () => {
    expect(diskText({ ...base, mode: 'copied' })).toBe('Holds 200 MB of its own.');
  });

  it('says what a linked base shares with the store install', () => {
    expect(diskText(base)).toBe('Holds 200 MB of its own and shares 16 GB with the store install, which takes no extra space.');
  });
});

describe('describeProblem', () => {
  it('explains each kind of damage in plain words', () => {
    expect(describeProblem(problem({ kind: 'missing' }))).toBe('The file is gone from the base.');
    expect(describeProblem(problem({ kind: 'content_changed' }))).toContain('no longer matches the hash');
    expect(describeProblem(problem({ kind: 'unexpected' }))).toContain('Something was written into it');
  });

  it('gives the expected and actual size for a size change', () => {
    const text = describeProblem(problem({ kind: 'size_changed', expectedBytes: 10 * 1024 ** 2, actualBytes: 7 * 1024 ** 2 }));
    expect(text).toBe('The file changed size (it should be 10 MB, it is 7.0 MB).');
  });

  it('warns when the damaged file is also a link to the store install', () => {
    expect(linkedWarning(problem({ linkedToStore: true }))).toContain('also reached the store install');
    expect(linkedWarning(problem({ linkedToStore: false }))).toBeNull();
  });
});

describe('verifySummary', () => {
  const reply = (over: Partial<VerifyReply>): VerifyReply => ({
    baseId: base.baseId,
    depth: 'quick',
    checked: 117,
    hashed: 0,
    clean: true,
    gameWrites: [],
    problems: [],
    ...over,
  });

  it('says a clean quick check checked sizes and times', () => {
    expect(verifySummary(reply({}))).toBe('Clean. Sizes and times were checked: 117 files checked, 0 hashed.');
  });

  it('says a clean full check hashed every file', () => {
    expect(verifySummary(reply({ depth: 'full', hashed: 117 }))).toBe('Clean. Every file was hashed again: 117 files checked, 117 hashed.');
  });

  it('counts the damaged files, singular and plural', () => {
    expect(verifySummary(reply({ clean: false, problems: [problem({})] }))).toContain('1 file needs attention');
    expect(verifySummary(reply({ clean: false, problems: [problem({}), problem({ path: 'b' })] }))).toContain('2 files need attention');
  });

  it('never calls a damaged base clean, even with no problems listed', () => {
    // The backend's own flag wins: a reply that says damaged is damaged.
    expect(verifySummary(reply({ clean: false, problems: [] }))).toMatch(/^Damaged/);
  });
});

describe('gameWritesNote', () => {
  it('is absent when the game wrote nothing', () => {
    expect(gameWritesNote([])).toBeNull();
  });

  it('lists the files the game wrote, and says they are not damage', () => {
    const note = gameWritesNote(['Data/Saves.ini']);
    expect(note).toContain('this file: Data/Saves.ini');
    expect(note).toContain('not damage');
  });
});

describe('build progress', () => {
  const progress = (over: Partial<BuildProgressEvent>): BuildProgressEvent => ({
    installId: 'steam-489830',
    filesDone: 0,
    filesTotal: 0,
    bytesHashed: 0,
    bytesTotal: 0,
    ...over,
  });

  it('measures by bytes when the total is known', () => {
    expect(progressPercent(progress({ bytesHashed: 1, bytesTotal: 4 }))).toBe(25);
  });

  it('never goes past 100 percent', () => {
    expect(progressPercent(progress({ bytesHashed: 9, bytesTotal: 4 }))).toBe(100);
  });

  it('falls back to files when no byte total is known, and to 0 with nothing at all', () => {
    expect(progressPercent(progress({ filesDone: 1, filesTotal: 4 }))).toBe(25);
    expect(progressPercent(progress({}))).toBe(0);
  });

  it('says how far the hashing has got', () => {
    expect(progressText(progress({ filesDone: 3, filesTotal: 10, bytesHashed: 1024, bytesTotal: 2048 })))
      .toBe('Hashed 3 of 10 files, 1.0 KB of 2.0 KB.');
  });
});

describe('damaged records', () => {
  it('says the record cannot be checked, and that removing it is the way forward', () => {
    expect(DAMAGED_EXPLANATION).toContain('cannot read this base');
    expect(DAMAGED_EXPLANATION).toContain('Agora cannot check the base');
    expect(DAMAGED_EXPLANATION).toContain('build the base again');
  });

  it('warns that a base on another drive keeps its folder, and where to look', () => {
    expect(DAMAGED_REMOVE_BODY).toContain('keeps its folder there');
    expect(DAMAGED_REMOVE_BODY).toContain('AgoraBases');
    expect(DAMAGED_REMOVE_BODY).toContain('not touched');
  });

  it('refuses removal of a damaged record that an instance uses, and allows it when none does', () => {
    expect(canRemove({ usedBy: [] })).toBe(true);
    expect(canRemove({ usedBy: [{ instanceId: 'a', name: 'Nordic Run' }] })).toBe(false);
  });
});
