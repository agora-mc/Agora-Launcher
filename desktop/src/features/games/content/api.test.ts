import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import {
  addInstanceContent,
  formatBytes,
  listInstanceContent,
  itemsForGame,
  listStore,
  moveInstanceContent,
  verifyStored,
  type StoreItem,
} from './api';

const storeItem: StoreItem = {
  itemId: '0151b2a0',
  name: 'Odin Patch',
  fileCount: 1,
  totalSize: 6772,
  sourceKind: 'archive',
  sourceLabel: 'Archive: odin.zip',
  addedAtUnixMs: 1,
  gameIds: [],
};

describe('listStore', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('returns the items the backend lists', async () => {
    invokeMock.mockResolvedValueOnce({ items: [storeItem], damaged: [] });
    await expect(listStore()).resolves.toEqual({ items: [storeItem], damaged: [] });
    expect(invokeMock).toHaveBeenCalledWith('games_content_list');
  });

  it('accepts an empty library as an empty list, not as a failure', async () => {
    invokeMock.mockResolvedValueOnce({ items: [], damaged: [] });
    await expect(listStore()).resolves.toEqual({ items: [], damaged: [] });
  });

  it.each([
    ['null (an unmocked command answers this)', null],
    ['a bare array', [storeItem]],
    ['an object without items', { other: [] }],
    ['items that are not a list', { items: 'Odin Patch' }],
    ['an item without an id', { items: [{ name: 'Odin Patch', sourceLabel: 'x', gameIds: [] }], damaged: [] }],
    ['an item without its games', { items: [{ ...storeItem, gameIds: undefined }], damaged: [] }],
    ['items without the damaged list', { items: [storeItem] }],
    ['a damaged record without a reason', { items: [], damaged: [{ itemId: 'abc' }] }],
  ])('rejects %s rather than reading it as an empty library', async (_label, reply) => {
    invokeMock.mockResolvedValueOnce(reply);
    await expect(listStore()).rejects.toThrow('games_content_list did not return a list of items.');
  });

  it('passes the backend error through when the command fails', async () => {
    invokeMock.mockRejectedValueOnce({ code: 'core_failed', message: 'The store index is damaged.' });
    await expect(listStore()).rejects.toMatchObject({ code: 'core_failed', message: 'The store index is damaged.' });
  });
});

describe('itemsForGame', () => {
  const byHand = { ...storeItem, itemId: 'a', gameIds: [] };
  const skyrimOnly = { ...storeItem, itemId: 'b', gameIds: ['skyrim-se'] };
  const fallout = { ...storeItem, itemId: 'c', gameIds: ['fallout-4'] };
  const both = { ...storeItem, itemId: 'd', gameIds: ['fallout-4', 'skyrim-se'] };

  it('keeps the items for the game and those that name no game', () => {
    const shown = itemsForGame([byHand, skyrimOnly, fallout, both], 'skyrim-se', false);
    expect(shown.map((item) => item.itemId)).toEqual(['a', 'b', 'd']);
  });

  it('shows everything when asked to', () => {
    expect(itemsForGame([byHand, skyrimOnly, fallout], 'skyrim-se', true)).toHaveLength(3);
  });
});

describe('instance content calls', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('rejects a content list that is not that list', async () => {
    invokeMock.mockResolvedValueOnce({ items: [] });
    await expect(listInstanceContent('inst-1')).rejects.toThrow('did not return an instance content list');
  });

  it('accepts a content list for the instance it asked for', async () => {
    invokeMock.mockResolvedValueOnce({ instanceId: 'inst-1', items: [] });
    await expect(listInstanceContent('inst-1')).resolves.toEqual({ instanceId: 'inst-1', items: [] });
    expect(invokeMock).toHaveBeenCalledWith('games_instance_content_list', { instanceId: 'inst-1' });
  });

  it('sends the move position under the name the Rust command takes', async () => {
    invokeMock.mockResolvedValueOnce(null);
    await moveInstanceContent('inst-1', '0151b2a0', 3);
    expect(invokeMock).toHaveBeenCalledWith('games_instance_content_move', {
      instanceId: 'inst-1',
      itemId: '0151b2a0',
      position: 3,
    });
  });

  it('sends an add with both ids and returns the outcome as the backend gave it', async () => {
    const outcome = { status: 'needs_installer', itemId: '0151b2a0', reason: 'a FOMOD installer', topLevel: ['fomod'] };
    invokeMock.mockResolvedValueOnce(outcome);
    await expect(addInstanceContent('inst-1', '0151b2a0')).resolves.toEqual(outcome);
    expect(invokeMock).toHaveBeenCalledWith('games_instance_content_add', {
      instanceId: 'inst-1',
      itemId: '0151b2a0',
    });
  });

  it('asks for a full check only when the user asks for one', async () => {
    invokeMock.mockResolvedValueOnce({ itemId: '0151b2a0', checked: 1, hashed: 0, problems: [] });
    await verifyStored('0151b2a0', false);
    expect(invokeMock).toHaveBeenLastCalledWith('games_content_verify', { itemId: '0151b2a0', full: false });
  });
});

describe('formatBytes', () => {
  it.each([
    [0, '0 bytes'],
    [1023, '1023 bytes'],
    [1536, '1.5 KB'],
    [10 * 1024 * 1024, '10 MB'],
    [5 * 1024 ** 3, '5.0 GB'],
  ])('shows %d bytes as %s', (bytes, text) => {
    expect(formatBytes(bytes)).toBe(text);
  });

  it.each([NaN, -1, Infinity])('does not invent a size for %s', (bytes) => {
    expect(formatBytes(bytes)).toBe('unknown size');
  });
});
