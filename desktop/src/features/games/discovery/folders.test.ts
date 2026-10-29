import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { addGameFolder, discoverGames, removeGameFolder } from './api';

describe('discoverGames', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('returns the folders and warnings the backend reports', async () => {
    const scan = {
      folders: [{ path: 'D:\\Games\\Skyrim', present: true }],
      warnings: [{ store: 'steam', message: 'appmanifest_489830.acf could not be read' }],
    };
    invokeMock.mockResolvedValueOnce(scan);
    await expect(discoverGames()).resolves.toEqual(scan);
    expect(invokeMock).toHaveBeenCalledWith('games_discover');
  });

  it.each([
    ['null (an unmocked command answers this)', null],
    ['a folder without a present flag', { folders: [{ path: 'D:\\Games' }], warnings: [] }],
    ['warnings that are not a list', { folders: [], warnings: 'steam' }],
    ['a warning without a message', { folders: [], warnings: [{ store: 'steam' }] }],
  ])('rejects %s rather than reading it as nothing found', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(discoverGames()).rejects.toThrow('games_discover did not return the folders and warnings.');
  });
});

describe('addGameFolder', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('sends the picked path and returns the folder core accepted', async () => {
    const added = { path: 'D:\\Games\\Skyrim', gameId: 'skyrim-se', gameName: 'Skyrim Special Edition' };
    invokeMock.mockResolvedValueOnce(added);
    await expect(addGameFolder('D:\\Games\\Skyrim')).resolves.toEqual(added);
    expect(invokeMock).toHaveBeenCalledWith('games_add_folder', { path: 'D:\\Games\\Skyrim' });
  });

  it('rejects a reply that does not confirm the folder', async () => {
    invokeMock.mockResolvedValueOnce({ path: 'D:\\Games\\Skyrim' });
    await expect(addGameFolder('D:\\Games\\Skyrim')).rejects.toThrow('did not confirm');
  });

  it('passes a refusal from core through unchanged', async () => {
    const refusal = { code: 'core_failed', message: 'No game Agora knows is in D:\\Docs.', coreCode: 'ERR_GAME_FOLDER_NOT_A_GAME' };
    invokeMock.mockRejectedValueOnce(refusal);
    await expect(addGameFolder('D:\\Docs')).rejects.toMatchObject({ coreCode: 'ERR_GAME_FOLDER_NOT_A_GAME' });
  });
});

describe('removeGameFolder', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('resolves with the path that was removed', async () => {
    invokeMock.mockResolvedValueOnce('D:\\Games\\Skyrim');
    await expect(removeGameFolder('d:\\games\\skyrim\\')).resolves.toBe('D:\\Games\\Skyrim');
    expect(invokeMock).toHaveBeenCalledWith('games_remove_folder', { path: 'd:\\games\\skyrim\\' });
  });

  it.each([
    ['null', null],
    ['an object', { removed: 'D:\\Games' }],
  ])('rejects %s rather than reading it as removed', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(removeGameFolder('D:\\Games')).rejects.toThrow('did not confirm the folder was removed');
  });
});
