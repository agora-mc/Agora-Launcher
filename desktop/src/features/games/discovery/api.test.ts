import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { listInstalledGames } from './api';
import { parseActiveGame } from '../shared/useGameShell';

describe('listInstalledGames', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('returns the games the backend lists', async () => {
    const games = [{ gameId: 'skyrim-se', name: 'Skyrim Special Edition', installs: [] }];
    invokeMock.mockResolvedValueOnce({ games });
    await expect(listInstalledGames()).resolves.toEqual({ games });
    expect(invokeMock).toHaveBeenCalledWith('games_list');
  });

  it.each([
    ['null (an unmocked command answers this)', null],
    ['a bare array', []],
    ['an object without games', { installs: [] }],
    ['a game without an id', { games: [{ name: 'Skyrim', installs: [] }] }],
    ['games that are not a list', { games: 'skyrim-se' }],
  ])('rejects %s rather than reading it as no games', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(listInstalledGames()).rejects.toThrow('games_list did not return a list of games.');
  });

  it('rejects when the command itself fails', async () => {
    invokeMock.mockRejectedValueOnce({ code: 'task_failed', message: 'Looking for installed games did not finish.' });
    await expect(listInstalledGames()).rejects.toMatchObject({ code: 'task_failed' });
  });
});

describe('parseActiveGame', () => {
  it('keeps a plausible game id', () => {
    expect(parseActiveGame('skyrim-se')).toBe('skyrim-se');
  });

  it.each([
    ['nothing stored', null],
    ['a non-string', 42],
    ['an empty string', ''],
    ['an over-long string', 'x'.repeat(65)],
  ])('falls back to Minecraft for %s', (_label, value) => {
    expect(parseActiveGame(value)).toBe('minecraft');
  });
});
