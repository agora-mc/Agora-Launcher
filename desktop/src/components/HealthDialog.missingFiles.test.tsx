import { describe, expect, it, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { HealthDialog } from './HealthDialog';
import type { HealthReport, InstanceDetail, InstalledMod } from '@/lib/tauri';

const tauriMocks = vi.hoisted(() => ({
  getInstanceDetail: vi.fn(),
  listRawModrinthVersions: vi.fn(),
  checkInstanceHealth: vi.fn(),
  disableModForTest: vi.fn(async () => undefined),
}));

vi.mock('@/lib/tauri', () => ({
  getInstanceDetail: tauriMocks.getInstanceDetail,
  listRawModrinthVersions: tauriMocks.listRawModrinthVersions,
  checkInstanceHealth: tauriMocks.checkInstanceHealth,
  disableModForTest: tauriMocks.disableModForTest,
  formatError: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));

// Stand in for the real install review: it only needs to show what it was
// asked to install.
const installFlowMock = vi.hoisted(() => ({ lastIntent: null as unknown }));
vi.mock('./InstallFlow', () => ({
  InstallFlow: ({ intent, onClose }: { intent: unknown; onClose: () => void }) => {
    installFlowMock.lastIntent = intent;
    return (
      <div role="dialog" aria-label="install review">
        <button onClick={onClose}>Close review</button>
      </div>
    );
  },
}));

vi.mock('@/lib/healthPreferences', () => ({
  loadHealthPreferences: async () => ({ mutedWarnings: [], muteAllRecommendations: false }),
  saveHealthPreferences: async () => undefined,
  activeHealthWarnings: (warnings: unknown[]) => warnings,
  healthWarningKey: (warning: { kind: string; filename?: string | null }) => `${warning.kind}:${warning.filename ?? ''}`,
}));

function mod(overrides: Partial<InstalledMod>): InstalledMod {
  return {
    filename: 'iris-1.8.0.jar',
    registry_id: null,
    modrinth_id: null,
    source: 'modrinth',
    version: '1.8.0',
    sha256: 'a'.repeat(64),
    installed_at: '2026-01-01T00:00:00Z',
    enabled: true,
    content_type: 'mod',
    ...overrides,
  };
}

function detailWith(mods: InstalledMod[]): InstanceDetail {
  return { manifest: { mods } } as unknown as InstanceDetail;
}

function report(missingFiles: string[] | undefined): HealthReport {
  return {
    score: 'yellow',
    scan_token: 't',
    blockers: [],
    recommendations: [],
    warnings: [
      {
        kind: 'manifest_drift',
        mod_id: null,
        filename: null,
        message: 'The instance manifest tracks 1 enabled mod file(s) that are absent from mods/.',
        suggested_action: 'Repair or reinstall the modpack before treating dependent mods as installed.',
      },
    ],
    ...(missingFiles === undefined ? {} : { missing_files: missingFiles }),
  };
}

function renderDialog(
  health: HealthReport,
  options: { onFindMod?: (query: string) => void } = {},
) {
  return render(
    <HealthDialog
      instanceId="inst-1"
      instanceName="My World"
      initialReport={health}
      onCancel={() => undefined}
      onSwitchLoader={async () => undefined}
      reviewOnly
      onFindMod={options.onFindMod}
    />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  installFlowMock.lastIntent = null;
  tauriMocks.checkInstanceHealth.mockResolvedValue(report([]));
});

describe('HealthDialog missing-file reinstall', () => {
  it('names each missing file and reinstalls a curated one through the install review', async () => {
    tauriMocks.getInstanceDetail.mockResolvedValue(
      detailWith([mod({ filename: 'fabric-api.jar', registry_id: 'fabric-api', version: '0.100.0' })]),
    );
    renderDialog(report(['fabric-api.jar']));

    expect(await screen.findByText('fabric-api.jar')).toBeInTheDocument();
    fireEvent.click(await screen.findByRole('button', { name: 'Reinstall' }));

    await screen.findByRole('dialog', { name: 'install review' });
    expect(installFlowMock.lastIntent).toEqual({
      action: { type: 'install', sourceType: 'curated', itemId: 'fabric-api', candidateVersion: '0.100.0' },
      targetInstance: 'inst-1',
      optionalDeps: { type: 'prompt' },
      requestedBy: 'interactive',
      overrides: { allowReplace: false, skipHealthScan: false, forceConflictResolution: {} },
    });
    // The health dialog steps aside while the review is open.
    expect(screen.queryByText('Health Check')).not.toBeInTheDocument();
  });

  it('maps a Modrinth-only entry to the Modrinth version id before reviewing', async () => {
    tauriMocks.getInstanceDetail.mockResolvedValue(
      detailWith([mod({ filename: 'iris-1.8.0.jar', modrinth_id: 'YL57xq9U', version: '1.8.0' })]),
    );
    tauriMocks.listRawModrinthVersions.mockResolvedValue([
      { version: '1.7.0', version_id: 'ver-old' },
      { version: '1.8.0', version_id: 'ver-new' },
    ]);
    renderDialog(report(['iris-1.8.0.jar']));

    fireEvent.click(await screen.findByRole('button', { name: 'Reinstall' }));
    await screen.findByRole('dialog', { name: 'install review' });

    expect(tauriMocks.listRawModrinthVersions).toHaveBeenCalledWith('inst-1', 'YL57xq9U', 'mod');
    expect(installFlowMock.lastIntent).toMatchObject({
      action: { sourceType: 'modrinth', itemId: 'YL57xq9U', candidateVersion: 'ver-new' },
    });
  });

  it('says so and opens no review when Modrinth no longer lists the recorded version', async () => {
    tauriMocks.getInstanceDetail.mockResolvedValue(
      detailWith([mod({ filename: 'iris-1.8.0.jar', modrinth_id: 'YL57xq9U', version: '1.8.0' })]),
    );
    tauriMocks.listRawModrinthVersions.mockResolvedValue([{ version: '1.9.0', version_id: 'ver-9' }]);
    renderDialog(report(['iris-1.8.0.jar']));

    fireEvent.click(await screen.findByRole('button', { name: 'Reinstall' }));

    expect(await screen.findByText(/no longer lists version 1\.8\.0/)).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: 'install review' })).not.toBeInTheDocument();
    expect(installFlowMock.lastIntent).toBeNull();
  });

  it('offers a search, not a reinstall, for an imported mod with no source', async () => {
    const onFindMod = vi.fn();
    tauriMocks.getInstanceDetail.mockResolvedValue(
      detailWith([mod({ filename: 'sodium-0.6.0.jar', source: 'manual' })]),
    );
    renderDialog(report(['sodium-0.6.0.jar']), { onFindMod });

    fireEvent.click(await screen.findByRole('button', { name: 'Find in Browse' }));
    expect(onFindMod).toHaveBeenCalledWith('sodium-0.6.0');
    expect(screen.queryByRole('button', { name: 'Reinstall' })).not.toBeInTheDocument();
  });

  it('offers no reinstall when the manifest cannot be read', async () => {
    tauriMocks.getInstanceDetail.mockRejectedValue(new Error('manifest unreadable'));
    const onFindMod = vi.fn();
    renderDialog(report(['fabric-api.jar']), { onFindMod });

    expect(await screen.findByRole('button', { name: 'Find in Browse' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Reinstall' })).not.toBeInTheDocument();
  });

  it('shows no file rows for an older report that has no missing_files field', async () => {
    renderDialog(report(undefined));
    await screen.findByText(/absent from mods/);
    expect(tauriMocks.getInstanceDetail).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'Reinstall' })).not.toBeInTheDocument();
  });

  it('re-scans the instance once the review closes', async () => {
    tauriMocks.getInstanceDetail.mockResolvedValue(
      detailWith([mod({ filename: 'fabric-api.jar', registry_id: 'fabric-api', version: '0.100.0' })]),
    );
    tauriMocks.checkInstanceHealth.mockResolvedValue(report([]));
    renderDialog(report(['fabric-api.jar']));

    fireEvent.click(await screen.findByRole('button', { name: 'Reinstall' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Close review' }));

    await waitFor(() => expect(tauriMocks.checkInstanceHealth).toHaveBeenCalledWith('inst-1'));
    await waitFor(() => expect(screen.queryByText('fabric-api.jar')).not.toBeInTheDocument());
  });
});
