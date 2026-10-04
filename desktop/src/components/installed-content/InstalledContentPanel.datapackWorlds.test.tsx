import { describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { InstalledContentPanel } from './InstalledContentPanel';
import type { InstalledContentRow } from '../../lib/tauri';

vi.mock('../../lib/tauri', () => ({
  formatError: (error: unknown) => String(error),
}));

function datapackRow(worldSync: InstalledContentRow['world_sync']): InstalledContentRow {
  return {
    key: 'datapack:veinminer-1.3.4.zip:abc',
    filename: 'veinminer-1.3.4.zip',
    display_name: 'VeinMiner',
    version: '1.3.4',
    content_type: 'datapack',
    enabled: true,
    installed_at: '2026-01-01T00:00:00Z',
    source: 'modrinth',
    source_label: 'Modrinth',
    update_pinned: false,
    source_url: null,
    registry_id: null,
    modrinth_id: 'proj',
    mod_jar_id: null,
    loader_mod_id: null,
    size_bytes: 1024,
    file_present: true,
    resolved_path: '/datapacks/veinminer-1.3.4.zip',
    author: 'Author',
    categories: [],
    icon_url: null,
    curation_status: 'unknown',
    agora_score: null,
    modrinth_downloads: null,
    metadata_status: 'unknown',
    world_sync: worldSync,
  } as InstalledContentRow;
}

const baseProps = {
  contentType: 'datapack' as const,
  addLabel: '+ Add Data Pack',
  locked: false,
  onAdd: vi.fn(),
  onToggle: vi.fn(async () => true),
  onBulkToggle: vi.fn(async () => true),
  onBulkRemove: vi.fn(() => true),
  onRemove: vi.fn(),
};

describe('InstalledContentPanel data pack worlds', () => {
  it('shows which worlds a data pack reaches and opens the chooser', () => {
    const onChooseWorlds = vi.fn();
    const row = datapackRow({ all_worlds: true, selected_worlds: [], available_worlds: ['A', 'B', 'C'], covered_worlds: 3 });
    render(<InstalledContentPanel {...baseProps} rows={[row]} onChooseWorlds={onChooseWorlds} />);
    expect(screen.getByTestId('datapack-world-status').textContent).toBe('All worlds (3)');
    fireEvent.click(screen.getByRole('button', { name: /Choose worlds for VeinMiner/i }));
    expect(onChooseWorlds).toHaveBeenCalledWith(row);
  });

  it('explains the new-world wait when there are no worlds yet', () => {
    const row = datapackRow({ all_worlds: true, selected_worlds: [], available_worlds: [], covered_worlds: 0 });
    render(<InstalledContentPanel {...baseProps} rows={[row]} />);
    expect(screen.getByTestId('datapack-world-status').textContent).toBe(
      'No worlds yet — added to new worlds from their second session',
    );
  });

  it('explains the sync on the tab and offers Sync now', () => {
    const onSyncWorlds = vi.fn();
    render(<InstalledContentPanel {...baseProps} rows={[]} onSyncWorlds={onSyncWorlds} />);
    expect(screen.getByText(/only loads data packs from inside each world/i)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Sync now' }));
    expect(onSyncWorlds).toHaveBeenCalled();
  });

  it('shows no world status on other content types', () => {
    const row = { ...datapackRow(undefined), content_type: 'mod', filename: 'sodium.jar' } as InstalledContentRow;
    render(<InstalledContentPanel {...baseProps} contentType="mod" rows={[row]} />);
    expect(screen.queryByTestId('datapack-world-status')).toBeNull();
  });
});
