import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import {
  checkGamePlugins,
  describeFailure,
  listGamePlugins,
  moveGamePlugin,
  setGamePluginActive,
  setGamePluginLocked,
  sortGamePlugins,
  type PluginsView,
} from './api';

const row = {
  position: 1,
  name: 'SkyUI.esp',
  implicit: false,
  active: true,
  present: true,
  managed: false,
  locked: false,
  headerRead: true,
  master: false,
  light: false,
  masters: ['Skyrim.esm'],
  headerError: null,
};

const view: PluginsView = {
  supported: true,
  rulesApply: true,
  listExists: true,
  rows: [row],
  findings: [],
};

describe('plugin commands', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('lists the plugins with the arguments the backend takes', async () => {
    invokeMock.mockResolvedValueOnce(view);
    await expect(listGamePlugins('inst-1')).resolves.toEqual(view);
    expect(invokeMock).toHaveBeenCalledWith('games_plugins_list', { instanceId: 'inst-1' });
  });

  it('passes the 1-based target position as `to`, the name the backend declares', async () => {
    invokeMock.mockResolvedValueOnce({
      report: { plugin: 'SkyUI.esp', from: 3, to: 2, written: true },
      plugins: view,
    });
    await moveGamePlugin('inst-1', 'SkyUI.esp', 2);
    expect(invokeMock).toHaveBeenCalledWith('games_plugins_move', { instanceId: 'inst-1', plugin: 'SkyUI.esp', to: 2 });
  });

  it('sends a sort as a dry run or for real, as it is asked', async () => {
    invokeMock.mockResolvedValueOnce({ report: { moves: [], blocked: [], written: false }, plugins: view });
    await sortGamePlugins('inst-1', true);
    expect(invokeMock).toHaveBeenLastCalledWith('games_plugins_sort', { instanceId: 'inst-1', dryRun: true });
  });

  it('sends the lock and active flags as they are asked', async () => {
    invokeMock.mockResolvedValue({ changed: true, plugins: view });
    await setGamePluginActive('inst-1', 'SkyUI.esp', false);
    expect(invokeMock).toHaveBeenLastCalledWith('games_plugins_set_active', {
      instanceId: 'inst-1',
      plugin: 'SkyUI.esp',
      active: false,
    });
    await setGamePluginLocked('inst-1', 'SkyUI.esp', true);
    expect(invokeMock).toHaveBeenLastCalledWith('games_plugins_set_locked', {
      instanceId: 'inst-1',
      plugin: 'SkyUI.esp',
      locked: true,
    });
  });

  it('reads a check into its status and findings', async () => {
    invokeMock.mockResolvedValueOnce({
      status: 'findings',
      findings: [{ message: 'Skyrim.esm is missing', refusesLaunch: true, plugins: ['SkyUI.esp'] }],
    });
    await expect(checkGamePlugins('inst-1')).resolves.toMatchObject({ status: 'findings' });
  });

  it.each([
    ['null (an unmocked command answers this)', null],
    ['a bare array', []],
    ['a list with no supported flag', { rows: [], findings: [] }],
    ['rows that are not a list', { ...view, rows: 'SkyUI.esp' }],
    ['a row with no name', { ...view, rows: [{ ...row, name: undefined }] }],
    ['a row whose active flag is a string', { ...view, rows: [{ ...row, active: 'yes' }] }],
    ['a finding with no plugin list', { ...view, findings: [{ message: 'x', refusesLaunch: false }] }],
  ])('rejects %s rather than reading it as an empty list', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(listGamePlugins('inst-1')).rejects.toThrow('games_plugins_list did not return the expected plugin data.');
  });

  it('rejects a check whose status is not one the tab knows', async () => {
    invokeMock.mockResolvedValueOnce({ status: 'fine', findings: [] });
    await expect(checkGamePlugins('inst-1')).rejects.toThrow('games_plugins_check did not return');
  });

  it('rejects a move that does not say where the plugin went', async () => {
    invokeMock.mockResolvedValueOnce({ report: { plugin: 'SkyUI.esp' }, plugins: view });
    await expect(moveGamePlugin('inst-1', 'SkyUI.esp', 2)).rejects.toThrow('games_plugins_move did not return');
  });

  it('lets a backend failure through unchanged', async () => {
    invokeMock.mockRejectedValueOnce({ code: 'core_failed', message: 'close the game', coreCode: 'ERR_X' });
    await expect(setGamePluginActive('inst-1', 'SkyUI.esp', true)).rejects.toMatchObject({ code: 'core_failed' });
  });
});

describe('describeFailure', () => {
  it('shows the backend message for a GamesError', () => {
    expect(describeFailure({ code: 'core_failed', message: 'close the game first' })).toBe('close the game first');
  });

  it.each([
    ['an Error', new Error('boom')],
    ['null', null],
    ['a message with no code', { message: 'boom' }],
  ])('falls back to a plain sentence for %s', (_label, failure) => {
    expect(describeFailure(failure)).toBe('Agora could not finish that. Try again.');
  });
});
