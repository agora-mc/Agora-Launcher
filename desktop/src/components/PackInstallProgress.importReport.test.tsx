/**
 * An Agora pack import is reviewed before anything is written, and a result that
 * did not end clean says so, mod by mod. Entries waiting on Modrinth can be
 * retried for this import alone, or with the setting turned on for good.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PackImportReports, PackInstallProvider, usePackInstall } from './PackInstallProgress';
import type { AgoraPackEntry, PackEntryOutcome, PackImportPlan, PackImportReport } from '../lib/tauri';

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
const plan: PackImportPlan = {
  pack_name: 'Friend Pack',
  instance: { id: 'friend-pack', name: 'Friend Pack', minecraft_version: '1.21', loader: 'fabric', loader_version: '0.16.0' },
  entries: [
    { name: 'sodium.jar', display_name: 'Sodium', source: 'curated', checkable: true, reason: null, action: null },
    { name: 'xaero.jar', display_name: null, source: 'modrinth', checkable: true, reason: 'Needs Modrinth, which is off.', action: 'enable_modrinth' },
    { name: 'ghost.jar', display_name: null, source: 'cannot_get', checkable: false, reason: "'ghost' is not in the catalog this install has.", action: null },
  ],
  file_sha256: 'b'.repeat(64),
  file_size: 1234,
  name_taken: false,
  suggested_name: 'Friend Pack',
  left_out: [],
  problems: [],
  soft_problems: [],
  notes: [],
};

function Starter({ path = 'C:\\friend.agora-pack.json' }: { path?: string }) {
  const { startPackFile } = usePackInstall();
  return <button onClick={() => startPackFile(path, path.split('\\').pop() ?? path)}>go</button>;
}

function renderImport(path?: string) {
  render(
    <PackInstallProvider>
      <Starter path={path} />
      <PackImportReports />
    </PackInstallProvider>,
  );
  fireEvent.click(screen.getByText('go'));
}

/** Mock answers for the commands the flow uses; tests override what they need. */
function answer(overrides: Record<string, (args: Record<string, unknown>) => unknown>) {
  invokeMock.mockImplementation(async (command: string, args: Record<string, unknown> = {}) => {
    if (overrides[command]) return overrides[command](args);
    if (command === 'plan_pack_import') return plan;
    if (command === 'import_instance_pack') return { instance_id: 'friend-pack', report: partial };
    if (command === 'set_content_provider_enabled') return [];
    if (command === 'retry_pack_import_entries') {
      return { ...partial, total: 1, installed: 1, outcomes: [{ ...installed('xaero.jar'), entry: xaeroEntry }] };
    }
    return null;
  });
}

const callsTo = (command: string) => invokeMock.mock.calls.filter(([name]) => name === command);

beforeEach(() => {
  vi.clearAllMocks();
  answer({});
});

describe('pack import review', () => {
  it('shows the pack and each mod before anything is imported, and imports only on Import', async () => {
    renderImport();

    const review = await screen.findByTestId('pack-import-review');
    expect(review.textContent).toContain('Import Friend Pack?');
    expect(review.textContent).toContain('1.21');
    expect(review.textContent).toContain('fabric 0.16.0');
    expect(screen.getByText('Sodium')).toBeTruthy();
    expect(screen.getByText(/From Modrinth/)).toBeTruthy();
    expect(screen.getByText(/'ghost' is not in the catalog/)).toBeTruthy();

    // The plan only reads: no import has been asked for yet.
    expect(callsTo('plan_pack_import')).toHaveLength(1);
    expect(callsTo('import_instance_pack')).toHaveLength(0);

    fireEvent.click(screen.getByRole('button', { name: 'Import' }));
    await waitFor(() => expect(callsTo('import_instance_pack')).toHaveLength(1));
    // The import is held to the exact bytes that were reviewed.
    expect(callsTo('import_instance_pack')[0][1]).toEqual({ sourcePath: 'C:\\friend.agora-pack.json', reviewedSha256: plan.file_sha256, name: 'Friend Pack', skipUnfetchable: false });
  });

  it('Cancel leaves nothing behind', async () => {
    renderImport();
    await screen.findByTestId('pack-import-review');
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByTestId('pack-import-review')).toBeNull();
    expect(callsTo('import_instance_pack')).toHaveLength(0);
  });

  it('a pack that cannot be read is named, and cannot be imported', async () => {
    answer({ plan_pack_import: () => Promise.reject({ code: 'ERR_PACK_PARSE', message: 'agora-pack missing loader.' }) });
    renderImport();
    expect(await screen.findByText('agora-pack missing loader.')).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Import' }) as HTMLButtonElement).disabled).toBe(true);
    expect(callsTo('import_instance_pack')).toHaveLength(0);
  });

  it('warns plainly when there is no connection, and only before anything is written', async () => {
    Object.defineProperty(navigator, 'onLine', { configurable: true, get: () => false });
    try {
      renderImport();
      const review = await screen.findByTestId('pack-import-review');
      // Only Sodium is ready to download now: Xaero waits on Modrinth, Ghost cannot be fetched.
      expect(review.textContent).toContain(
        "This mod needs to be downloaded. Couldn't connect: no internet, or the server can't be reached right now. Nothing was installed.",
      );
    } finally {
      Object.defineProperty(navigator, 'onLine', { configurable: true, get: () => true });
    }
  });

  it('a .mrpack is reviewed too, and imports under the free name the review suggested', async () => {
    answer({ plan_pack_import: () => ({ ...plan, suggested_name: 'Friend Pack (2)', name_taken: true }) });
    renderImport('C:\\packs\\friend.mrpack');
    await screen.findByTestId('pack-import-review');
    expect(callsTo('plan_pack_import')[0][1]).toEqual({ sourcePath: 'C:\\packs\\friend.mrpack' });
    expect(callsTo('import_instance_pack')).toHaveLength(0);
    expect((screen.getByLabelText('Instance name') as HTMLInputElement).value).toBe('Friend Pack (2)');

    fireEvent.click(screen.getByRole('button', { name: 'Import as a copy' }));
    await waitFor(() => expect(callsTo('import_instance_pack')).toHaveLength(1));
    expect(callsTo('import_instance_pack')[0][1]).toEqual({
      sourcePath: 'C:\\packs\\friend.mrpack',
      reviewedSha256: plan.file_sha256,
      name: 'Friend Pack (2)',
      skipUnfetchable: false,
    });
  });

  it('Import anyway sends the skip flag, and cancelling a review drops the staged copy', async () => {
    answer({ plan_pack_import: () => ({ ...plan, soft_problems: ['mods/ghost.jar has no way to be fetched'] }) });
    renderImport('C:\\packs\\friend.mrpack');
    fireEvent.click(await screen.findByRole('button', { name: 'Import anyway' }));
    await waitFor(() => expect(callsTo('import_instance_pack')).toHaveLength(1));
    expect(callsTo('import_instance_pack')[0][1]).toMatchObject({ skipUnfetchable: true });
    expect(callsTo('discard_pack_review')).toHaveLength(0);
  });

  it('cancelling the review removes the staged copy', async () => {
    renderImport('C:\\packs\\friend.mrpack');
    await screen.findByTestId('pack-import-review');
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(callsTo('discard_pack_review')).toHaveLength(1));
    expect(callsTo('discard_pack_review')[0][1]).toEqual({ sha256: plan.file_sha256 });
    expect(callsTo('import_instance_pack')).toHaveLength(0);
  });

  it('imports under the name the user typed over the suggestion', async () => {
    renderImport('C:\\packs\\friend.mrpack');
    const field = await screen.findByLabelText('Instance name');
    fireEvent.change(field, { target: { value: '  Sam\'s pack ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Import' }));
    await waitFor(() => expect(callsTo('import_instance_pack')).toHaveLength(1));
    expect(callsTo('import_instance_pack')[0][1]).toMatchObject({ name: "Sam's pack" });
  });
});

describe('pack import report', () => {
  beforeEach(() => {
    answer({ import_instance_pack: () => ({ instance_id: 'friend-pack', report: partial }) });
  });

  it('shows what was not installed, then retries only the waiting entries for this import, leaving the setting alone', async () => {
    renderImport();
    fireEvent.click(await screen.findByRole('button', { name: 'Import' }));

    expect(await screen.findByText('4 of 5 mods installed.')).toBeTruthy();
    expect(screen.getByText(/Needs Modrinth, which is off/)).toBeTruthy();
    // The catalog name is shown when known.
    expect(screen.getByText('Xaero Minimap')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Allow Modrinth for this import' }));

    expect(await screen.findByText('5 of 5 mods installed.')).toBeTruthy();
    expect(callsTo('set_content_provider_enabled')).toHaveLength(0);
    const retry = callsTo('retry_pack_import_entries');
    expect(retry).toHaveLength(1);
    expect(retry[0][1]).toEqual({ instanceId: 'friend-pack', entries: [xaeroEntry], allowModrinthOnce: true });
  });

  it('turns the Modrinth setting on for good only when asked to, then retries', async () => {
    renderImport();
    fireEvent.click(await screen.findByRole('button', { name: 'Import' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Turn on Modrinth search (stays on, Settings > Services)' }));

    expect(await screen.findByText('5 of 5 mods installed.')).toBeTruthy();
    expect(callsTo('set_content_provider_enabled')[0][1]).toEqual({ providerId: 'modrinth', enabled: true });
    expect(callsTo('retry_pack_import_entries')[0][1]).toMatchObject({ allowModrinthOnce: false });
  });

  it('shows nothing extra for a clean, fully checked import', async () => {
    answer({
      import_instance_pack: () => ({
        instance_id: 'friend-pack',
        report: { ...partial, total: 1, installed: 1, outcomes: [installed('a.jar')] },
      }),
    });
    renderImport();
    fireEvent.click(await screen.findByRole('button', { name: 'Import' }));
    await waitFor(() => expect(screen.getByText('Pack installed successfully.')).toBeTruthy());
    expect(screen.queryByTestId('pack-import-summary')).toBeNull();
  });

  it('can be dismissed with its close control', async () => {
    renderImport();
    fireEvent.click(await screen.findByRole('button', { name: 'Import' }));
    await screen.findByTestId('pack-import-summary');
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss import results' }));
    expect(screen.queryByTestId('pack-import-summary')).toBeNull();
  });
});

describe('pack import without a connection', () => {
  const offlineReport = (total: number, installedCount: number): PackImportReport => {
    const offlineEntry = (filename: string): PackEntryOutcome => ({
      name: filename, status: 'needs_action', verified_against_pack: false, action: null,
      reason: "This mod needs to be downloaded. Couldn't connect: no internet, or the server can't be reached right now. Try again when you're online.",
      entry: entry(filename, 'sodium'),
    });
    const outcomes = [
      ...Array.from({ length: installedCount }, (_, i) => installed(`done-${i}.jar`)),
      ...Array.from({ length: total - installedCount }, (_, i) => offlineEntry(`wait-${i}.jar`)),
    ];
    return { instance_id: 'friend-pack', instance_name: 'Friend Pack', total, installed: installedCount, warnings: [], outcomes };
  };

  it('says the mods need a connection, that nothing was installed, and offers Try again without Settings', async () => {
    answer({ import_instance_pack: () => ({ instance_id: 'friend-pack', report: offlineReport(3, 0) }) });
    renderImport();
    fireEvent.click(await screen.findByRole('button', { name: 'Import' }));

    const summary = await screen.findByTestId('pack-import-summary');
    expect(summary.textContent).toContain(
      "These 3 mods need to be downloaded. Couldn't connect: no internet, or the server can't be reached right now. Nothing was installed. Try again when you're online.",
    );
    expect(screen.getByRole('button', { name: 'Try again' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Open Settings' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Allow Modrinth for this import' })).toBeNull();
  });

  it('does not claim nothing was installed when some mods did install', async () => {
    answer({ import_instance_pack: () => ({ instance_id: 'friend-pack', report: offlineReport(3, 1) }) });
    renderImport();
    fireEvent.click(await screen.findByRole('button', { name: 'Import' }));

    const summary = await screen.findByTestId('pack-import-summary');
    expect(summary.textContent).toContain("These 2 mods need to be downloaded. Couldn't connect: no internet, or the server can't be reached right now.");
    expect(summary.textContent).not.toContain('Nothing was installed');
  });

  it('a removed import reports the connection problem as the failure, not a generic one', async () => {
    answer({
      import_instance_pack: () => Promise.reject({
        code: 'ERR_PACK_OFFLINE',
        message: "These 3 mods need to be downloaded. Couldn't connect: no internet, or the server can't be reached right now. Nothing was installed, and the new instance was removed. Try again when you're online.",
      }),
    });
    renderImport();
    fireEvent.click(await screen.findByRole('button', { name: 'Import' }));
    expect(await screen.findByText(/the new instance was removed/)).toBeTruthy();
  });
});
