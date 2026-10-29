import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { buildBase, listBases, removeBase, verifyBase } from './api';

const base = {
  baseId: 'skyrim-se_steam_1.6.1170_nobuild',
  gameId: 'skyrim-se',
  store: 'steam',
  version: '1.6.1170',
  build: null,
  mode: 'linked',
  location: 'D:/Games/AgoraBases/skyrim-se_steam_1.6.1170_nobuild',
  present: true,
  fileCount: 117,
  linkedFileCount: 40,
  linkedBytes: 16_000_000_000,
  copiedBytes: 197_000_000,
  skippedCount: 1,
  createdUnixMs: 1700000000000,
  usedBy: [{ instanceId: 'nordic-run', name: 'Nordic Run' }],
};

const verifyClean = {
  baseId: base.baseId,
  depth: 'quick',
  checked: 117,
  hashed: 0,
  clean: true,
  gameWrites: [],
  problems: [],
};

describe('listBases', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('returns the bases the backend lists, for this game', async () => {
    invokeMock.mockResolvedValueOnce({ bases: [base] });
    await expect(listBases('skyrim-se')).resolves.toEqual({ bases: [base] });
    expect(invokeMock).toHaveBeenCalledWith('games_base_list', { gameId: 'skyrim-se' });
  });

  it('accepts an empty list as a real answer', async () => {
    invokeMock.mockResolvedValueOnce({ bases: [] });
    await expect(listBases('skyrim-se')).resolves.toEqual({ bases: [] });
  });

  // A reply that is not the documented shape must reject, never read as "no bases".
  it.each([
    ['null (an unmocked command answers this)', null],
    ['a bare array', [base]],
    ['an object without bases', { items: [base] }],
    ['bases that is not a list', { bases: null }],
    ['a base with a mode it does not know', { bases: [{ ...base, mode: 'hybrid' }] }],
    ['a base without its size', { bases: [{ ...base, copiedBytes: undefined }] }],
    ['a base whose users are not named', { bases: [{ ...base, usedBy: [{ instanceId: 'x' }] }] }],
    ['a size that is not a number', { bases: [{ ...base, linkedBytes: '16 GB' }] }],
  ])('rejects %s', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(listBases('skyrim-se')).rejects.toThrow('games_base_list did not return the documented shape.');
  });

  it('passes a backend error through unchanged', async () => {
    const error = { code: 'instances_unreadable', message: 'Agora could not read the instances.' };
    invokeMock.mockRejectedValueOnce(error);
    await expect(listBases('skyrim-se')).rejects.toEqual(error);
  });
});

describe('buildBase', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('sends the mode the user chose, and never includes excluded files by default', async () => {
    invokeMock.mockResolvedValueOnce({ status: 'existing', baseId: base.baseId, mode: 'copied', location: base.location, fileCount: 117 });
    await buildBase('steam-489830', 'copied');
    expect(invokeMock).toHaveBeenCalledWith('games_base_build', {
      installId: 'steam-489830',
      mode: 'copied',
      includeExcluded: false,
    });
  });

  it('accepts a built base, an existing base, and a link choice', async () => {
    invokeMock.mockResolvedValueOnce({
      status: 'built', baseId: base.baseId, mode: 'linked', location: base.location,
      fileCount: 117, linkedBytes: 16_000_000_000, copiedBytes: 197_000_000,
    });
    await expect(buildBase('steam-489830', 'linked')).resolves.toMatchObject({ status: 'built' });

    invokeMock.mockResolvedValueOnce({ status: 'link_unavailable', reason: 'another volume', copiedBytesNeeded: 16_000_000_000 });
    await expect(buildBase('steam-489830', 'linked')).resolves.toEqual({
      status: 'link_unavailable', reason: 'another volume', copiedBytesNeeded: 16_000_000_000,
    });
  });

  it.each([
    ['an empty object', {}],
    ['a status it does not know', { status: 'queued', baseId: 'x' }],
    ['a built reply without its byte counts', { status: 'built', baseId: 'x', mode: 'linked', location: 'y', fileCount: 1 }],
    ['a link choice without the size', { status: 'link_unavailable', reason: 'x' }],
  ])('rejects %s', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(buildBase('steam-489830', 'linked')).rejects.toThrow('games_base_build did not return the documented shape.');
  });
});

describe('verifyBase', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('asks for a quick or a full check', async () => {
    invokeMock.mockResolvedValueOnce({ ...verifyClean, depth: 'full', hashed: 117 });
    await verifyBase(base.baseId, true);
    expect(invokeMock).toHaveBeenCalledWith('games_base_verify', { baseId: base.baseId, full: true });
  });

  it('returns the named problems of a damaged base', async () => {
    const damaged = {
      ...verifyClean,
      clean: false,
      problems: [
        { path: 'Data/Skyrim - Meshes0.bsa', kind: 'size_changed', expectedBytes: 10, actualBytes: 7, linkedToStore: true },
      ],
    };
    invokeMock.mockResolvedValueOnce(damaged);
    await expect(verifyBase(base.baseId, false)).resolves.toEqual(damaged);
  });

  it.each([
    ['a reply with no clean flag', { ...verifyClean, clean: undefined }],
    ['a problem with an unknown kind', { ...verifyClean, clean: false, problems: [{ path: 'a', kind: 'vanished', expectedBytes: null, actualBytes: null, linkedToStore: false }] }],
    ['a clean answer with no depth', { ...verifyClean, depth: undefined }],
    ['a null reply', null],
  ])('rejects %s instead of reading it as clean', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(verifyBase(base.baseId, false)).rejects.toThrow('games_base_verify did not return the documented shape.');
  });
});

describe('removeBase', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('accepts a removed base', async () => {
    invokeMock.mockResolvedValueOnce({ baseId: base.baseId, status: 'removed' });
    await expect(removeBase(base.baseId)).resolves.toEqual({ baseId: base.baseId, status: 'removed' });
    expect(invokeMock).toHaveBeenCalledWith('games_base_remove', { baseId: base.baseId });
  });

  it.each([
    ['null (an unmocked command answers this)', null],
    ['a reply that does not say removed', { baseId: base.baseId, status: 'ok' }],
    ['an empty object', {}],
  ])('rejects %s instead of reading it as removed', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(removeBase(base.baseId)).rejects.toThrow('games_base_remove did not return the documented shape.');
  });
});
