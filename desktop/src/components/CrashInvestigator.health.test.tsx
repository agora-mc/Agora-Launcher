/**
 * The Crash Doctor opens with the instance's own mod problems. A crash that a
 * missing dependency explains should say so first, with the same Reinstall
 * the Health Check offers, and a failed health read must not look like "fine".
 */
import { describe, expect, it, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { CrashInvestigator } from './CrashInvestigator';
import type { HealthReport, InstanceDetail, InstalledMod } from '@/lib/tauri';

const tauriMocks = vi.hoisted(() => ({
  checkInstanceHealth: vi.fn(),
  getInstanceDetail: vi.fn(),
  investigateCrash: vi.fn(),
  listRawModrinthVersions: vi.fn(),
}));

// Anything the Doctor reads besides the mocked calls below must not reach a
// real Tauri bridge; an unexpected call resolves to nothing.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }));

vi.mock('@/lib/tauri', async () => {
  const actual = await vi.importActual<typeof import('@/lib/tauri')>('@/lib/tauri');
  return {
    ...actual,
    checkInstanceHealth: tauriMocks.checkInstanceHealth,
    getInstanceDetail: tauriMocks.getInstanceDetail,
    investigateCrash: tauriMocks.investigateCrash,
    listRawModrinthVersions: tauriMocks.listRawModrinthVersions,
  };
});

vi.mock('../lib/tauri', async () => {
  const actual = await vi.importActual<typeof import('../lib/tauri')>('../lib/tauri');
  return {
    ...actual,
    checkInstanceHealth: tauriMocks.checkInstanceHealth,
    getInstanceDetail: tauriMocks.getInstanceDetail,
    investigateCrash: tauriMocks.investigateCrash,
    listRawModrinthVersions: tauriMocks.listRawModrinthVersions,
  };
});

// The review is covered by its own tests; here it only has to show what it
// was asked to install.
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

function mod(overrides: Partial<InstalledMod>): InstalledMod {
  return {
    filename: 'fabric-api.jar',
    registry_id: 'fabric-api',
    modrinth_id: null,
    source: 'curated',
    version: '0.100.0',
    sha256: 'a'.repeat(64),
    installed_at: '2026-01-01T00:00:00Z',
    enabled: true,
    content_type: 'mod',
    ...overrides,
  } as InstalledMod;
}

function report(overrides: Partial<HealthReport> = {}): HealthReport {
  return {
    score: 'red',
    scan_token: 't',
    warnings: [],
    recommendations: [],
    blockers: [],
    ...overrides,
  } as HealthReport;
}

const missingProviderBlocker = {
  kind: 'missing_required_dependency',
  mod_id: 'fabric-api',
  filename: 'sodium-0.6.0.jar',
  message: 'sodium-0.6.0 needs Fabric API, whose file is missing.',
  suggested_action: 'Reinstall Fabric API from the Health Check.',
  missing_file: 'fabric-api.jar',
};

function renderDoctor() {
  return render(
    <CrashInvestigator
      instanceId="inst-1"
      crashFilename="crash.txt"
      onClose={() => undefined}
      onLaunch={async () => ({ started: false } as never)}
      processState={{ status: 'idle' } as never}
    />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  installFlowMock.lastIntent = null;
  tauriMocks.investigateCrash.mockResolvedValue({
    fingerprint: null,
    signature_name: null,
    suspects: [],
    suggested_action: { kind: 'NoSuspects' },
    ruled_out: [],
  });
  tauriMocks.getInstanceDetail.mockResolvedValue({ manifest: { mods: [mod({})] } } as unknown as InstanceDetail);
});

describe('Crash Doctor: mod problems first', () => {
  it('lists the health blockers before anything else, with the Health Check fix', async () => {
    tauriMocks.checkInstanceHealth.mockResolvedValue(report({ blockers: [missingProviderBlocker] }));
    renderDoctor();

    expect(await screen.findByText(/Before digging into the crash: 1 mod problem to fix/)).toBeInTheDocument();
    expect(screen.getByText('sodium-0.6.0 needs Fabric API, whose file is missing.')).toBeInTheDocument();
    const reinstall = await screen.findByRole('button', { name: 'Reinstall' });
    fireEvent.click(reinstall);
    await screen.findByRole('dialog', { name: 'install review' });
    expect(installFlowMock.lastIntent).toMatchObject({
      action: { type: 'install', sourceType: 'curated', itemId: 'fabric-api', candidateVersion: '0.100.0' },
      targetInstance: 'inst-1',
    });
  });

  it('says so when the health read fails, rather than showing no problems', async () => {
    tauriMocks.checkInstanceHealth.mockRejectedValue(new Error('registry locked'));
    renderDoctor();

    expect(await screen.findByRole('alert')).toHaveTextContent(/couldn.t check this instance/);
    expect(screen.queryByText(/Before digging into the crash/)).not.toBeInTheDocument();
  });

  it('shows no problem section when the instance is healthy', async () => {
    tauriMocks.checkInstanceHealth.mockResolvedValue(report({ score: 'green' }));
    renderDoctor();

    expect(await screen.findByText('What we checked')).toBeInTheDocument();
    expect(screen.queryByText(/Before digging into the crash/)).not.toBeInTheDocument();
  });
});
