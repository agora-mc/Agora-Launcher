import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MigrationReportPanel } from './MigrationReportPanel';
import type { MigrationPlan, MigrationReport, ModMigrationEntry } from '@/lib/tauri';

const tauriMocks = vi.hoisted(() => ({
  getMigrationReport: vi.fn(),
  planVersionMigration: vi.fn(),
  runVersionMigration: vi.fn(),
  listManifestMcVersions: vi.fn(),
}));

vi.mock('@/lib/tauri', () => ({
  getMigrationReport: tauriMocks.getMigrationReport,
  planVersionMigration: tauriMocks.planVersionMigration,
  runVersionMigration: tauriMocks.runVersionMigration,
  listManifestMcVersions: tauriMocks.listManifestMcVersions,
  formatError: (error: unknown) => String(error),
}));

function entry(filename: string): ModMigrationEntry {
  return {
    filename,
    display_name: filename.replace('.jar', ''),
    modrinth_id: null,
    registry_id: null,
    content_type: 'mod',
    installed_version: '1.0.0',
    status: 'ready',
  };
}

function allReadyReport(): MigrationReport {
  return {
    instance_id: 'alpha',
    source_version: '1.21.1',
    target_version: '1.20.1',
    loader: 'fabric',
    summary: {
      total: 3,
      ready: 3,
      not_yet: 0,
      abandoned: 0,
      superseded: 0,
      unknown: 0,
      unclassifiable: 0,
    },
    verdict: 'ready',
    mods: [entry('a.jar'), entry('b.jar'), entry('c.jar')],
    warnings: [],
  };
}

function planFor(report: MigrationReport): MigrationPlan {
  return {
    instanceId: 'alpha',
    sourceVersion: '1.21.1',
    targetVersion: '1.20.1',
    loader: 'fabric',
    sourceLoaderVersion: '0.16.0',
    targetLoaderVersion: '0.15.11',
    swaps: report.mods.map((mod) => ({
      oldFilename: mod.filename,
      contentType: 'mod',
      oldEnabled: true,
      newFilename: mod.filename.replace('.jar', '-1.20.1.jar'),
      target: {
        version_id: `${mod.filename}-target`,
        version_number: '2.0.0',
        filename: mod.filename.replace('.jar', '-1.20.1.jar'),
        download_url: 'https://example.invalid/mod.jar',
      },
    })),
    blockers: [],
    warnings: [],
    fingerprint: 'fp',
    instanceStateHash: 'hash',
    report,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  tauriMocks.listManifestMcVersions.mockResolvedValue(['1.21.1', '1.20.1', '1.19.2']);
  tauriMocks.getMigrationReport.mockResolvedValue(allReadyReport());
  tauriMocks.planVersionMigration.mockImplementation(async () => planFor(allReadyReport()));
});

describe('MigrationReportPanel', () => {
  it('offers the stored version list rather than asking the user to spell one', async () => {
    render(<MigrationReportPanel instanceId="alpha" currentVersion="1.21.1" loader="fabric" />);
    const select = await screen.findByLabelText('Target Minecraft version');
    await waitFor(() => expect(
      screen.getByRole('option', { name: '1.20.1' }),
    ).toBeInTheDocument());

    // Scoped to the instance's loader: a version it has no build for would
    // only ever produce a report saying so.
    expect(tauriMocks.listManifestMcVersions).toHaveBeenCalledWith('fabric');
    // The version you are already on is not somewhere to move to.
    expect(screen.queryByRole('option', { name: '1.21.1' })).not.toBeInTheDocument();
    expect(select).toBeInstanceOf(HTMLSelectElement);
  });

  it('states the count once when every mod is ready', async () => {
    render(<MigrationReportPanel instanceId="alpha" currentVersion="1.21.1" loader="fabric" />);
    fireEvent.change(await screen.findByLabelText('Target Minecraft version'), {
      target: { value: '1.20.1' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Check' }));

    await waitFor(() => expect(
      screen.getByText('Everything has a build for the target version.'),
    ).toBeInTheDocument());

    // The verdict above and the disclosure below already carry it; "3 ready of
    // 3" in between was the same number a third time.
    expect(screen.queryByText(/3 ready.*of 3/)).not.toBeInTheDocument();
    expect(screen.getByText(/Ready \(3\)/)).toBeInTheDocument();
  });

  it('still breaks the counts down when the report is mixed', async () => {
    const mixed = allReadyReport();
    mixed.summary = { ...mixed.summary, ready: 2, not_yet: 1 };
    mixed.verdict = 'not_yet';
    mixed.mods = [entry('a.jar'), entry('b.jar'), { ...entry('c.jar'), status: 'not_yet' }];
    tauriMocks.getMigrationReport.mockResolvedValue(mixed);
    tauriMocks.planVersionMigration.mockResolvedValue(null);

    render(<MigrationReportPanel instanceId="alpha" currentVersion="1.21.1" loader="fabric" />);
    fireEvent.change(await screen.findByLabelText('Target Minecraft version'), {
      target: { value: '1.20.1' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Check' }));

    // Here the breakdown is the whole point, so it stays.
    await waitFor(() => expect(
      screen.getByText(/2 ready, 1 with no build yet of 3\./),
    ).toBeInTheDocument());
  });
});
