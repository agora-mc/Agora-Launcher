import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());
const listenMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }));

import {
  checkGameInstance,
  createGameInstance,
  gamesErrorCode,
  gamesErrorMessage,
  getGameInstance,
  launchGameInstance,
  listGameInstances,
  onCreateProgress,
  onInstanceExited,
  setGameInstanceDeployment,
} from './api';

const card = {
  instanceId: 'grounded-1',
  gameId: 'skyrim-se',
  name: 'Grounded',
  store: 'steam',
  version: '1.6.1170',
  pinned: true,
  deployment: null,
  lastLaunchedAt: null,
  running: false,
};

const framework = {
  kind: 'framework',
  blocks: true,
  title: 'Script extender',
  detail: 'It needs skse64.dll, but the game has no file of its family.',
  why: 'A framework built for another game version often stops the game from starting.',
  repair: 'Install the matching extender.',
};

describe('listGameInstances', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('passes the game id and returns the cards', async () => {
    invokeMock.mockResolvedValueOnce({ instances: [card] });
    await expect(listGameInstances('skyrim-se')).resolves.toEqual({ instances: [card] });
    expect(invokeMock).toHaveBeenCalledWith('games_instance_list', { gameId: 'skyrim-se' });
  });

  it('returns an empty list only when the reply says so', async () => {
    invokeMock.mockResolvedValueOnce({ instances: [] });
    await expect(listGameInstances('skyrim-se')).resolves.toEqual({ instances: [] });
  });

  it.each([
    ['null (an unmocked command answers this)', null],
    ['a bare array', [card]],
    ['an object without instances', { cards: [card] }],
    ['a card without a name', { instances: [{ ...card, name: undefined }] }],
    ['a card whose deployment is not a mode', { instances: [{ ...card, deployment: 'everything' }] }],
    ['a card whose pinned flag is text', { instances: [{ ...card, pinned: 'yes' }] }],
  ])('rejects %s rather than reading it as no instances', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(listGameInstances('skyrim-se')).rejects.toThrow('games_instance_list did not return what the Instances page expects.');
  });

  it('passes a rejected command through as the rejection', async () => {
    invokeMock.mockRejectedValueOnce({ code: 'invalid_request', message: 'Minecraft instances are listed under My Instances, not here.' });
    await expect(listGameInstances('minecraft')).rejects.toMatchObject({ code: 'invalid_request' });
  });
});

describe('getGameInstance', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('rejects a page reply that has no saves choice', async () => {
    invokeMock.mockResolvedValueOnce({ instanceId: 'grounded-1', name: 'Grounded', pinned: true, running: false, saves: 'everyone' });
    await expect(getGameInstance('grounded-1')).rejects.toThrow('games_instance_get did not return');
  });

  it('accepts a complete page reply', async () => {
    const detail = {
      instanceId: 'grounded-1',
      gameId: 'skyrim-se',
      gameName: 'Skyrim Special Edition',
      name: 'Grounded',
      pinned: true,
      store: 'steam',
      version: '1.6.1170',
      build: null,
      baseId: 'base-1',
      installId: null,
      unpinnedReason: null,
      deployment: 'links',
      saves: 'shared',
      createdAt: '2026-10-01 10:00:00',
      lastLaunchedAt: null,
      running: false,
    };
    invokeMock.mockResolvedValueOnce(detail);
    await expect(getGameInstance('grounded-1')).resolves.toEqual(detail);
    expect(invokeMock).toHaveBeenCalledWith('games_instance_get', { instanceId: 'grounded-1' });
  });
});

describe('checkGameInstance', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('accepts findings with their kind and whether they block', async () => {
    const check = { instanceId: 'grounded-1', checked: true, status: 'findings', findings: [framework] };
    invokeMock.mockResolvedValueOnce(check);
    await expect(checkGameInstance('grounded-1')).resolves.toEqual(check);
  });

  it.each([
    ['an unknown finding kind', { ...framework, kind: 'mystery' }],
    ['a finding without a repair field', { ...framework, repair: undefined, detail: 7 }],
    ['a finding that is null', null],
  ])('rejects %s', async (_label, finding) => {
    invokeMock.mockResolvedValueOnce({ instanceId: 'grounded-1', checked: true, status: 'findings', findings: [finding] });
    await expect(checkGameInstance('grounded-1')).rejects.toThrow('games_instance_check did not return');
  });

  it('rejects a check whose status is not one the page knows', async () => {
    invokeMock.mockResolvedValueOnce({ instanceId: 'grounded-1', checked: true, status: 'fine', findings: [] });
    await expect(checkGameInstance('grounded-1')).rejects.toThrow('games_instance_check did not return');
  });
});

describe('launchGameInstance', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('sends the launch-anyway flag and the one-off deployment, and null when there is none', async () => {
    invokeMock.mockResolvedValueOnce({ status: 'launched', pid: 4242, deployment: 'links', notice: null, findings: [], reason: null, next: null });
    await launchGameInstance('grounded-1', { launchAnyway: false });
    expect(invokeMock).toHaveBeenLastCalledWith('games_instance_launch', {
      instanceId: 'grounded-1',
      launchAnyway: false,
      deployment: null,
    });

    invokeMock.mockResolvedValueOnce({ status: 'launched', pid: 4243, deployment: 'copies', notice: null, findings: [], reason: null, next: null });
    await launchGameInstance('grounded-1', { launchAnyway: true, deployment: 'copies' });
    expect(invokeMock).toHaveBeenLastCalledWith('games_instance_launch', {
      instanceId: 'grounded-1',
      launchAnyway: true,
      deployment: 'copies',
    });
  });

  it('returns a refused launch with its findings, and a fallback question with its next rung', async () => {
    invokeMock.mockResolvedValueOnce({ status: 'refused', pid: null, deployment: null, notice: null, findings: [framework], reason: null, next: null });
    await expect(launchGameInstance('grounded-1', { launchAnyway: false })).resolves.toMatchObject({ status: 'refused' });

    invokeMock.mockResolvedValueOnce({ status: 'vfs_unavailable', pid: null, deployment: null, notice: null, findings: [], reason: 'no driver', next: 'links' });
    await expect(launchGameInstance('grounded-1', { launchAnyway: false })).resolves.toMatchObject({ status: 'vfs_unavailable', next: 'links' });
  });

  it.each([
    ['a status nobody defined', { status: 'maybe', pid: null, deployment: null, notice: null, findings: [], reason: null, next: null }],
    ['a pid that is text', { status: 'launched', pid: '4242', deployment: null, notice: null, findings: [], reason: null, next: null }],
    ['a fallback rung that is not a mode', { status: 'vfs_unavailable', pid: null, deployment: null, notice: null, findings: [], reason: 'x', next: 'direct' }],
    ['no reply at all', undefined],
  ])('rejects %s rather than reading it as a launch', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(launchGameInstance('grounded-1', { launchAnyway: false })).rejects.toThrow('games_instance_launch did not return');
  });
});

describe('create, delete and deployment wrappers', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('sends the install, name and base mode the backend reads', async () => {
    invokeMock.mockResolvedValueOnce({ instanceId: 'grounded-1', name: 'Grounded', base: 'pinned', reusedBase: true, baseId: 'b', unpinnedReason: null });
    await createGameInstance('skyrim-se', 'steam-489830', 'Grounded', 'linked');
    expect(invokeMock).toHaveBeenCalledWith('games_instance_create', {
      gameId: 'skyrim-se',
      installId: 'steam-489830',
      name: 'Grounded',
      baseMode: 'linked',
    });
  });

  it('rejects a deployment change that answers with a mode the page does not know', async () => {
    invokeMock.mockResolvedValueOnce({ instanceId: 'grounded-1', deployment: 'direct' });
    await expect(setGameInstanceDeployment('grounded-1', 'links')).rejects.toThrow('games_instance_set_deployment did not return');
  });

  it('accepts an automatic deployment as null', async () => {
    invokeMock.mockResolvedValueOnce({ instanceId: 'grounded-1', deployment: null });
    await expect(setGameInstanceDeployment('grounded-1', 'auto')).resolves.toEqual({ instanceId: 'grounded-1', deployment: null });
    expect(invokeMock).toHaveBeenCalledWith('games_instance_set_deployment', { instanceId: 'grounded-1', mode: 'auto' });
  });
});

describe('events', () => {
  beforeEach(() => {
    listenMock.mockReset();
  });

  it('hands the progress and exit payloads to the page', async () => {
    const seen: unknown[] = [];
    listenMock.mockImplementation((_name: string, handler: (event: { payload: unknown }) => void) => {
      handler({ payload: { filesDone: 50, filesTotal: 100, bytesHashed: 1, bytesTotal: 2 } });
      return Promise.resolve(() => undefined);
    });
    await onCreateProgress((progress) => seen.push(progress));
    expect(listenMock).toHaveBeenCalledWith('games-instance-create-progress', expect.any(Function));
    expect(seen).toEqual([{ filesDone: 50, filesTotal: 100, bytesHashed: 1, bytesTotal: 2 }]);

    const exits: unknown[] = [];
    listenMock.mockImplementation((_name: string, handler: (event: { payload: unknown }) => void) => {
      handler({ payload: { instanceId: 'grounded-1', restoreError: 'hash mismatch' } });
      return Promise.resolve(() => undefined);
    });
    await onInstanceExited((exited) => exits.push(exited));
    expect(listenMock).toHaveBeenCalledWith('games-instance-exited', expect.any(Function));
    expect(exits).toEqual([{ instanceId: 'grounded-1', restoreError: 'hash mismatch' }]);
  });
});

describe('error helpers', () => {
  it('reads the message and code from a GamesError, and says so when there is neither', () => {
    const error = { code: 'instance_running', message: "'grounded-1' is running." };
    expect(gamesErrorMessage(error)).toBe("'grounded-1' is running.");
    expect(gamesErrorCode(error)).toBe('instance_running');
    expect(gamesErrorMessage(new Error('boom'))).toBe('boom');
    expect(gamesErrorMessage(undefined)).toBe('Something went wrong, and Agora did not say what.');
    expect(gamesErrorCode('plain string')).toBeNull();
  });
});
