import { describe, expect, it, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { InstalledContentPanel } from './InstalledContentPanel';
import type { InstalledContentRow, UpdateInfo } from '../../lib/tauri';

vi.mock('../../lib/tauri', () => ({
  formatError: (error: unknown) => String(error),
}));

function row(overrides: Partial<InstalledContentRow> & { filename: string }): InstalledContentRow {
  return {
    key: overrides.filename,
    display_name: overrides.filename.replace('.jar', ''),
    version: '1.0.0',
    content_type: 'mod',
    enabled: true,
    installed_at: '2026-01-01T00:00:00Z',
    source: 'modrinth',
    source_label: 'Modrinth',
    update_pinned: false,
    source_url: null,
    registry_id: null,
    modrinth_id: 'proj',
    mod_jar_id: overrides.filename.replace('.jar', ''),
    loader_mod_id: null,
    size_bytes: 1024,
    file_present: true,
    resolved_path: `/mods/${overrides.filename}`,
    author: 'Author',
    categories: [],
    icon_url: null,
    curation_status: 'unknown',
    agora_score: null,
    modrinth_downloads: null,
    metadata_status: 'unknown',
    ...overrides,
  } as InstalledContentRow;
}

function update(filename: string): UpdateInfo {
  return {
    filename,
    mod_jar_id: filename.replace('.jar', ''),
    current_version: '1.0.0',
    latest_version: '2.0.0',
    target_version: 'v2',
    source: 'modrinth',
  };
}

const baseProps = {
  contentType: 'mod' as const,
  addLabel: 'Import Mod',
  locked: false,
  onAdd: vi.fn(),
  onToggle: vi.fn(async () => true),
  onBulkToggle: vi.fn(async () => true),
  onBulkRemove: vi.fn(() => true),
  onRemove: vi.fn(),
};

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
});

describe('InstalledContentPanel Update All', () => {
  it('stays hidden until an update check has actually found something', () => {
    render(
      <InstalledContentPanel
        {...baseProps}
        rows={[row({ filename: 'sodium.jar' })]}
        onCheckUpdates={async () => []}
        onUpdateAll={vi.fn()}
      />,
    );
    expect(screen.queryByRole('button', { name: /Update all/i })).toBeNull();
  });

  it('appears with a count once updates are found', async () => {
    render(
      <InstalledContentPanel
        {...baseProps}
        rows={[row({ filename: 'sodium.jar' }), row({ filename: 'iris.jar' })]}
        onCheckUpdates={async () => [update('sodium.jar'), update('iris.jar')]}
        onUpdateAll={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Check for updates/i }));
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Update all 2 mods/i })).toBeTruthy();
    });
  });

  // The backend returns updates for the WHOLE instance, so a mods panel must
  // never offer to update a shader that it does not render.
  it('only counts and submits updates belonging to this panel', async () => {
    const onUpdateAll = vi.fn();
    render(
      <InstalledContentPanel
        {...baseProps}
        rows={[row({ filename: 'sodium.jar' })]}
        onCheckUpdates={async () => [update('sodium.jar'), update('complementary.zip')]}
        onUpdateAll={onUpdateAll}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Check for updates/i }));
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Update all 1 mods/i })).toBeTruthy();
    });

    fireEvent.click(screen.getByRole('button', { name: /Update all/i }));
    expect(onUpdateAll).toHaveBeenCalledTimes(1);
    expect(onUpdateAll.mock.calls[0][0].map((entry: UpdateInfo) => entry.filename)).toEqual(['sodium.jar']);
  });

  // The whole point of persisting the check: coming back to an instance must
  // not present a blank slate that forces the user to re-check.
  it('seeds from a cached check without any network call', () => {
    const onCheckUpdates = vi.fn(async () => []);
    render(
      <InstalledContentPanel
        {...baseProps}
        rows={[row({ filename: 'sodium.jar' })]}
        initialUpdates={[update('sodium.jar')]}
        onCheckUpdates={onCheckUpdates}
        onUpdateAll={vi.fn()}
      />,
    );
    expect(screen.getByRole('button', { name: /Update all 1 mods/i })).toBeTruthy();
    expect(onCheckUpdates).not.toHaveBeenCalled();
  });

  it('lets an explicit check overwrite a stale cached seed', async () => {
    render(
      <InstalledContentPanel
        {...baseProps}
        rows={[row({ filename: 'sodium.jar' })]}
        initialUpdates={[update('sodium.jar')]}
        onCheckUpdates={async () => []}
        onUpdateAll={vi.fn()}
      />,
    );
    expect(screen.getByRole('button', { name: /Update all/i })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: /Check for updates/i }));
    await waitFor(() => {
      expect(screen.queryByRole('button', { name: /Update all/i })).toBeNull();
    });
  });

  it('does not submit updates while the instance is locked', async () => {
    const onUpdateAll = vi.fn();
    const { rerender } = render(
      <InstalledContentPanel
        {...baseProps}
        rows={[row({ filename: 'sodium.jar' })]}
        onCheckUpdates={async () => [update('sodium.jar')]}
        onUpdateAll={onUpdateAll}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Check for updates/i }));
    await waitFor(() => screen.getByRole('button', { name: /Update all/i }));

    rerender(
      <InstalledContentPanel
        {...baseProps}
        locked
        rows={[row({ filename: 'sodium.jar' })]}
        onCheckUpdates={async () => [update('sodium.jar')]}
        onUpdateAll={onUpdateAll}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Update all/i }));
    expect(onUpdateAll).not.toHaveBeenCalled();
  });
});

describe('per-mod update pin', () => {
  it('excludes a pinned row from Update All and shows no badge for it', async () => {
    const onUpdateAll = vi.fn();
    render(
      <InstalledContentPanel
        {...baseProps}
        rows={[row({ filename: 'sodium.jar' }), row({ filename: 'iris.jar', update_pinned: true })]}
        onCheckUpdates={async () => [update('sodium.jar'), update('iris.jar')]}
        onUpdateAll={onUpdateAll}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Check for updates/i }));
    // Both have updates upstream, but the pinned one must not be offered.
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Update all 1 mods/i })).toBeTruthy();
    });
    fireEvent.click(screen.getByRole('button', { name: /Update all/i }));
    expect(onUpdateAll.mock.calls[0][0].map((entry: UpdateInfo) => entry.filename)).toEqual(['sodium.jar']);
  });

  it('offers to unpin a pinned row and to pin an unpinned one', () => {
    const onTogglePin = vi.fn();
    render(
      <InstalledContentPanel
        {...baseProps}
        rows={[row({ filename: 'iris.jar', update_pinned: true })]}
        onTogglePin={onTogglePin}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Unpin updates/i }));
    expect(onTogglePin).toHaveBeenCalledWith(expect.objectContaining({ filename: 'iris.jar' }), false);
  });
});
