/**
 * Catalogue detail for the selected item.
 *
 * The rules that matter: Agora's curated prose wins when it exists, Modrinth is
 * the fallback AND the only source of categories, and a failing lookup must
 * degrade to an empty panel rather than take the panel down with it.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';

const mocks = vi.hoisted(() => ({
  getRegistryItem: vi.fn(),
  fetchModrinthProject: vi.fn(),
  isModrinthEnabled: vi.fn(),
}));

vi.mock('@/lib/tauri', () => ({
  getRegistryItem: mocks.getRegistryItem,
  fetchModrinthProject: mocks.fetchModrinthProject,
  isModrinthEnabled: mocks.isModrinthEnabled,
}));

const { readContentDetail } = await import('./readAdapters');

const registryItem = (over: Record<string, unknown> = {}) => ({
  id: 'r1', name: 'Thing', description: 'Agora words', page_url: 'https://agora/x', ...over,
});
const modrinthProject = (over: Record<string, unknown> = {}) => ({
  id: 'm1', title: 'Thing', description: 'Modrinth words',
  categories: ['optimization', 'utility'], page_url: 'https://modrinth/x', ...over,
});

beforeEach(() => {
  mocks.getRegistryItem.mockReset();
  mocks.fetchModrinthProject.mockReset();
  mocks.isModrinthEnabled.mockReset().mockResolvedValue(true);
});

describe('readContentDetail', () => {
  it('prefers the curated Agora description', async () => {
    mocks.getRegistryItem.mockResolvedValue(registryItem());
    mocks.fetchModrinthProject.mockResolvedValue(modrinthProject());
    const out = await readContentDetail('r1', 'm1');
    expect(out.description).toBe('Agora words');
    expect(out.source).toBe('agora');
  });

  it('still takes categories from Modrinth, which is the only place they exist', async () => {
    mocks.getRegistryItem.mockResolvedValue(registryItem());
    mocks.fetchModrinthProject.mockResolvedValue(modrinthProject());
    const out = await readContentDetail('r1', 'm1');
    expect(out.categories).toEqual(['optimization', 'utility']);
  });

  it('falls back to Modrinth prose when Agora has none', async () => {
    mocks.getRegistryItem.mockResolvedValue(registryItem({ description: null }));
    mocks.fetchModrinthProject.mockResolvedValue(modrinthProject());
    const out = await readContentDetail('r1', 'm1');
    expect(out.description).toBe('Modrinth words');
  });

  it('never calls Modrinth when the integration is off', async () => {
    mocks.getRegistryItem.mockResolvedValue(registryItem());
    mocks.isModrinthEnabled.mockResolvedValue(false);
    const out = await readContentDetail('r1', 'm1');
    expect(mocks.fetchModrinthProject).not.toHaveBeenCalled();
    expect(out.categories).toEqual([]);
    expect(out.description).toBe('Agora words');
  });

  it('a failing lookup yields an empty result, never a rejection', async () => {
    mocks.getRegistryItem.mockRejectedValue(new Error('registry down'));
    mocks.fetchModrinthProject.mockRejectedValue(new Error('modrinth down'));
    const out = await readContentDetail('r1', 'm1');
    expect(out).toEqual({ description: null, categories: [], pageUrl: null, source: null });
  });

  it('asks nothing when the item has no catalogue identity', async () => {
    const out = await readContentDetail(null, null);
    expect(mocks.getRegistryItem).not.toHaveBeenCalled();
    expect(mocks.fetchModrinthProject).not.toHaveBeenCalled();
    expect(out.source).toBeNull();
  });
});
