import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { installFomod, previewFomod, showFomod } from './api';

describe('showFomod', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('asks for the installer with its instance and the defaults flag', async () => {
    const shown = {
      installer: { itemId: 'aaaa', title: 'Pack', steps: [] },
      suggestedChoices: [],
    };
    invokeMock.mockResolvedValueOnce(shown);
    await expect(showFomod('aaaa', 'inst-1', true)).resolves.toEqual(shown);
    expect(invokeMock).toHaveBeenCalledWith('games_fomod_show', {
      contentId: 'aaaa',
      instanceId: 'inst-1',
      useDefaults: true,
    });
  });

  it.each([
    ['null (an unmocked command answers this)', null],
    ['an installer with no steps list', { installer: { itemId: 'aaaa' } }],
    ['a bare installer', { itemId: 'aaaa', steps: [] }],
  ])('rejects %s rather than showing an empty installer', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(showFomod('aaaa', null, false)).rejects.toThrow('games_fomod_show did not return an installer.');
  });

  it('passes the core refusal through to the caller', async () => {
    invokeMock.mockRejectedValueOnce({
      code: 'core_failed',
      message: 'a step depends on a version Agora cannot check',
    });
    await expect(showFomod('aaaa', null, true)).rejects.toMatchObject({
      message: 'a step depends on a version Agora cannot check',
    });
  });
});

describe('preview and install', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('sends the same command with preview on, and changes nothing', async () => {
    invokeMock.mockResolvedValueOnce({ status: 'preview', files: [] });
    const choices = [{ step: 'Look', group: 'Body', plugins: ['Slim'] }];
    await previewFomod('aaaa', null, choices);
    expect(invokeMock).toHaveBeenCalledWith('games_fomod_install', {
      contentId: 'aaaa',
      instanceId: null,
      choices,
      preview: true,
    });
  });

  it('sends the install with preview off', async () => {
    invokeMock.mockResolvedValueOnce({ status: 'installed' });
    await installFomod('aaaa', 'inst-1', []);
    expect(invokeMock).toHaveBeenCalledWith('games_fomod_install', {
      contentId: 'aaaa',
      instanceId: 'inst-1',
      choices: [],
      preview: false,
    });
  });
});
