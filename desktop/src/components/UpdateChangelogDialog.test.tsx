import { describe, expect, it, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { UpdateChangelogDialog } from './UpdateChangelogDialog';
import type { UpdateInfo, VersionChangelog } from '@/lib/tauri';

const tauriMocks = vi.hoisted(() => ({
  getUpdateChangelogs: vi.fn(),
}));

vi.mock('@/lib/tauri', () => ({
  getUpdateChangelogs: tauriMocks.getUpdateChangelogs,
  formatError: (error: unknown) => String(error),
}));

function update(name: string): UpdateInfo {
  return {
    filename: `${name}.jar`,
    mod_jar_id: name,
    current_version: '1.0.0',
    latest_version: '2.0.0',
    target_version: 'v2',
    source: 'modrinth',
  };
}

function changelog(itemId: string, version: string, body: string): VersionChangelog {
  return { item_id: itemId, version, changelog: body, published_at: null, source: 'modrinth_id' };
}

beforeEach(() => {
  vi.clearAllMocks();
  tauriMocks.getUpdateChangelogs.mockResolvedValue([]);
});

describe('UpdateChangelogDialog', () => {
  it('asks the registry for the range between installed and candidate', async () => {
    render(
      <UpdateChangelogDialog
        updates={[update('sodium')]}
        displayNameFor={() => 'Sodium'}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    await waitFor(() => {
      expect(tauriMocks.getUpdateChangelogs).toHaveBeenCalledWith('sodium', '1.0.0', '2.0.0');
    });
  });

  // An item with no published changelog is the normal case for curated and
  // self-hosted mods; it must read as information, not as a failure.
  it('says so plainly when nothing is published', async () => {
    render(
      <UpdateChangelogDialog
        updates={[update('sodium')]}
        displayNameFor={() => 'Sodium'}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    await waitFor(() => {
      expect(screen.getByText(/No changelog is published/i)).toBeTruthy();
    });
  });

  it('renders changelog markdown as text, not raw HTML', async () => {
    tauriMocks.getUpdateChangelogs.mockResolvedValue([
      changelog('sodium', '2.0.0', '## Fixed\n\nA crash on load'),
    ]);
    render(
      <UpdateChangelogDialog
        updates={[update('sodium')]}
        displayNameFor={() => 'Sodium'}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    await waitFor(() => {
      expect(screen.getByText('A crash on load')).toBeTruthy();
    });
    expect(screen.getByRole('heading', { name: 'Fixed' })).toBeTruthy();
  });

  it('confirms with the caller after review', async () => {
    const onConfirm = vi.fn();
    render(
      <UpdateChangelogDialog
        updates={[update('sodium')]}
        displayNameFor={() => 'Sodium'}
        onConfirm={onConfirm}
        onCancel={vi.fn()}
      />,
    );
    await waitFor(() => screen.getByRole('button', { name: 'Update' }));
    fireEvent.click(screen.getByRole('button', { name: 'Update' }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  // A registry read failing must not prevent the user updating — the changelog
  // is advisory, the update is the point.
  it('still allows the update when the changelog lookup fails', async () => {
    tauriMocks.getUpdateChangelogs.mockRejectedValue(new Error('registry unavailable'));
    const onConfirm = vi.fn();
    render(
      <UpdateChangelogDialog
        updates={[update('sodium')]}
        displayNameFor={() => 'Sodium'}
        onConfirm={onConfirm}
        onCancel={vi.fn()}
      />,
    );
    await waitFor(() => {
      expect(screen.getByText(/Could not load the changelog/i)).toBeTruthy();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Update' }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('starts a batch collapsed and expands one entry on demand', async () => {
    tauriMocks.getUpdateChangelogs.mockResolvedValue([
      changelog('sodium', '2.0.0', 'Sodium notes'),
    ]);
    render(
      <UpdateChangelogDialog
        updates={[update('sodium'), update('iris')]}
        displayNameFor={(u) => u.mod_jar_id}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    await waitFor(() => screen.getByRole('button', { name: /Update all 2/i }));
    expect(screen.queryByText('Sodium notes')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: /sodium/i }));
    await waitFor(() => {
      expect(screen.getByText('Sodium notes')).toBeTruthy();
    });
  });
});
