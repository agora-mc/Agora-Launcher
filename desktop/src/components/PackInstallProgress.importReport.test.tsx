/**
 * An Agora pack import that does not end clean says so, mod by mod, and the
 * entries that were waiting on Modrinth can be retried without re-importing.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PackInstallProvider, usePackInstall } from './PackInstallProgress';
import type { AgoraPackEntry, PackEntryOutcome, PackImportReport } from '../lib/tauri';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

const sha = 'a'.repeat(64);
const entry = (filename: string, registry: string | null): AgoraPackEntry => ({
  filename, version: '1', registry_id: registry, modrinth_id: registry ? null : 'xaero', sha256: sha, source: registry ? 'registry' : 'modrinth_raw',
});
const installed = (filename: string): PackEntryOutcome => ({
  name: filename, status: 'installed', verified_against_pack: true, reason: null, action: null, entry: entry(filename, 'sodium'),
});
const xaeroEntry = entry('xaero.jar', null);
const waiting: PackEntryOutcome = {
  name: 'xaero.jar', status: 'needs_action', verified_against_pack: false, action: 'enable_modrinth', display_name: 'Xaero Minimap',
  reason: 'Needs Modrinth, which is off. Turning it on sends requests for this mod to Modrinth.',
  entry: xaeroEntry,
};
const partial: PackImportReport = {
  instance_id: 'friend-pack', instance_name: 'Friend Pack', total: 5, installed: 4, warnings: [],
  outcomes: [waiting, installed('a.jar'), installed('b.jar'), installed('c.jar'), installed('d.jar')],
};

function Starter() {
  const { startPackFile } = usePackInstall();
  return <button onClick={() => startPackFile('C:\friend.agora-pack.json', 'friend.agora-pack.json')}>go</button>;
}

beforeEach(() => {
  vi.clearAllMocks();
  invokeMock.mockImplementation(async (command: string) => {
    if (command === 'import_instance_pack') return { instance_id: 'friend-pack', report: partial };
    if (command === 'set_content_provider_enabled') return [];
    if (command === 'retry_pack_import_entries') {
      return {
        ...partial, total: 1, installed: 1,
        outcomes: [{ ...installed('xaero.jar'), entry: xaeroEntry }],
      };
    }
    return null;
  });
});

describe('pack import report', () => {
  it('shows what was not installed, then retries only the waiting entries once Modrinth is on', async () => {
    render(<PackInstallProvider><Starter /></PackInstallProvider>);
    fireEvent.click(screen.getByText('go'));

    expect(await screen.findByText('4 of 5 mods installed.')).toBeTruthy();
    expect(screen.getByText(/Needs Modrinth, which is off/)).toBeTruthy();
    // The catalog name is shown when known.
    expect(screen.getByText('Xaero Minimap')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Turn on Modrinth for this' }));

    expect(await screen.findByText('5 of 5 mods installed.')).toBeTruthy();
    expect(invokeMock).toHaveBeenCalledWith('set_content_provider_enabled', { providerId: 'modrinth', enabled: true });
    const retry = invokeMock.mock.calls.find(([command]) => command === 'retry_pack_import_entries');
    expect(retry?.[1]).toEqual({ instanceId: 'friend-pack', entries: [xaeroEntry] });
  });

  it('shows nothing extra for a clean, fully checked import', async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command !== 'import_instance_pack') return null;
      return {
        instance_id: 'friend-pack',
        report: { ...partial, total: 1, installed: 1, outcomes: [installed('a.jar')] },
      };
    });
    render(<PackInstallProvider><Starter /></PackInstallProvider>);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() => expect(screen.getByText('Pack installed successfully.')).toBeTruthy());
    expect(screen.queryByTestId('pack-import-summary')).toBeNull();
  });
});
