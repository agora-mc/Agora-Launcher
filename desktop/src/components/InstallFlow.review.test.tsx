/**
 * What the install review says before anything is written.
 *
 * Covers three review fixes: the already-installed line names the dependency
 * (it used to say only "One required dependency is already installed"), an
 * optional extra that cannot be found cannot be ticked, and the restore-point
 * line states what the snapshot really covers.
 */
import { describe, expect, it, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { InstallFlow } from './InstallFlow';
import type {
  DepDisposition,
  InstallIntent,
  ProgressEvent,
  ResolvedDep,
  ResolvedInstallPlan,
} from '../lib/installFlow';

const installFlowMocks = vi.hoisted(() => ({
  resolveInstallPlan: vi.fn(),
  applyInstallPlan: vi.fn(),
  subscribeProgress: vi.fn(),
  cancelInstall: vi.fn(),
  planNeedsUserReview: vi.fn(() => true),
}));

vi.mock('../lib/installFlow', async () => {
  const actual = await vi.importActual<typeof import('../lib/installFlow')>('../lib/installFlow');
  return {
    ...actual,
    resolveInstallPlan: installFlowMocks.resolveInstallPlan,
    applyInstallPlan: installFlowMocks.applyInstallPlan,
    subscribeProgress: installFlowMocks.subscribeProgress,
    cancelInstall: installFlowMocks.cancelInstall,
    planNeedsUserReview: installFlowMocks.planNeedsUserReview,
  };
});

vi.mock('../lib/tauri', async () => {
  const actual = await vi.importActual<typeof import('../lib/tauri')>('../lib/tauri');
  return {
    ...actual,
    getSetting: vi.fn(async () => false),
    formatError: (e: unknown) => String(e),
    parseLauncherError: (e: unknown) => ({ message: String(e), code: 'ERR_TEST' }),
    restoreSnapshot: vi.fn(async () => {}),
    getCachedInstanceUpdates: vi.fn(async () => []),
    clearCachedInstanceUpdates: vi.fn(async () => {}),
  };
});

vi.mock('../features/tour/tourSignals', () => ({ emitTourSignal: vi.fn() }));

const intent: InstallIntent = {
  action: { type: 'batch-install', items: [{ itemId: 'sodium', candidateVersion: '0.6.0', sourceType: 'curated' }] },
  targetInstance: 'test-inst',
  optionalDeps: { type: 'prompt' },
  requestedBy: 'interactive',
  overrides: { allowReplace: false, skipHealthScan: false, forceConflictResolution: {} },
};

function dep(
  modJarId: string,
  disposition: DepDisposition,
  overrides: Partial<ResolvedDep> = {},
): ResolvedDep {
  return {
    modJarId,
    requirement: 'required',
    source: 'manifest',
    disposition,
    displayName: null,
    pageUrl: null,
    requestedBy: [],
    ...overrides,
  };
}

const reused = (modJarId: string): DepDisposition => ({
  type: 'reuse-existing',
  modJarId,
  installedFilename: `${modJarId}.jar`,
});

function planWith(dependencies: ResolvedDep[]): ResolvedInstallPlan {
  return {
    fingerprint: 'fp-review-123',
    intent,
    operation: { type: 'batch-install', operations: [] },
    dependencies,
    conflicts: [],
    filesToAdd: [],
    filesToRemove: [],
    filesToDisable: [],
    snapshot: { label: 'test', estimatedBytes: 0 },
    diskEstimate: {
      downloadBytes: 0, snapshotBytes: 0, applyOverheadBytes: 0,
      peakAdditionalBytes: 0, postCommitDeltaBytes: 0,
    },
    warnings: [],
    blockingErrors: [],
    pendingChoices: [],
    createdAt: new Date().toISOString(),
    instanceStateHash: 'hash',
    registryRevision: 'rev',
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  installFlowMocks.subscribeProgress.mockImplementation(
    async (_id: string, _handler: (e: ProgressEvent) => void) => () => {},
  );
  installFlowMocks.planNeedsUserReview.mockReturnValue(true);
});

const panel = () => screen.getByRole('dialog', { name: /Review Instance Changes/i });

async function renderReview(dependencies: ResolvedDep[]) {
  installFlowMocks.resolveInstallPlan.mockResolvedValue(planWith(dependencies));
  render(<InstallFlow intent={intent} instanceName="Test Instance" open onClose={vi.fn()} />);
  await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'true'));
  return panel();
}

describe('install review: already-installed line', () => {
  it('names the one required dependency that is already there', async () => {
    const review = await renderReview([
      dep('fabric-api', reused('fabric-api'), { displayName: 'Fabric API' }),
    ]);
    expect(within(review).getByText('Fabric API is already installed.')).toBeInTheDocument();
  });

  it('names up to three satisfied dependencies in one sentence', async () => {
    const review = await renderReview([
      dep('fabric-api', reused('fabric-api'), { displayName: 'Fabric API' }),
      dep('sodium', reused('sodium'), { displayName: 'Sodium' }),
    ]);
    expect(within(review).getByText('Fabric API and Sodium are already installed.')).toBeInTheDocument();
  });

  it('falls back to the mod id when the display name is blank', async () => {
    const review = await renderReview([
      dep('fabric-api', reused('fabric-api'), { displayName: '   ' }),
    ]);
    expect(within(review).getByText('fabric-api is already installed.')).toBeInTheDocument();
  });

  it('switches to a count past three so the line stays short', async () => {
    const review = await renderReview(
      ['a', 'b', 'c', 'd'].map((id) => dep(id, reused(id), { displayName: id.toUpperCase() })),
    );
    expect(within(review).getByText('4 required dependencies are already installed.')).toBeInTheDocument();
  });
});

describe('install review: optional extras', () => {
  it('cannot tick an optional extra that could not be found', async () => {
    const review = await renderReview([
      dep('forge-config-screens', { type: 'unresolved', reason: 'Failed to list Modrinth versions' }, {
        requirement: 'optional',
        displayName: 'Forge Config Screens',
      }),
    ]);
    const box = within(review).getByRole('checkbox', {
      name: 'Include optional dependency Forge Config Screens',
    });
    expect(box).toBeDisabled();
    expect(within(review).getByText(/Could not be found/)).toBeInTheDocument();
  });

  it('keeps an extra that can be installed tickable', async () => {
    const review = await renderReview([
      dep('open-parties', { type: 'install-candidate', artifact: {} as never }, {
        requirement: 'optional',
        displayName: 'Open Parties and Claims',
      }),
    ]);
    const box = within(review).getByRole('checkbox', {
      name: 'Include optional dependency Open Parties and Claims',
    });
    expect(box).toBeEnabled();
  });
});

describe('install review: restore point copy', () => {
  it('says what the restore point covers and where to roll back from', async () => {
    const review = await renderReview([]);
    const copy = within(review).getByText(/automatic restore point/);
    expect(copy.textContent).toContain('worlds');
    expect(copy.textContent).toContain('not logs or screenshots');
    expect(copy.textContent).toContain('Snapshots tab');
    expect(copy.textContent).not.toContain('one click');
  });
});

describe('install review: a required mod the jar declares', () => {
  const missing = (): ResolvedDep => dep('fabric-api', { type: 'unresolved', reason: 'it is not in the Agora catalog' }, {
    source: 'jar',
    displayName: 'Fabric API',
    requestedBy: ['xaeros-minimap'],
  });

  it('says what needs it', async () => {
    const review = await renderReview([
      dep('fabric-api', { type: 'install-candidate', artifact: {} as never }, {
        source: 'jar',
        displayName: 'Fabric API',
        requestedBy: ["Xaero's Minimap"],
      }),
    ]);
    expect(within(review).getByText("Needed by Xaero's Minimap")).toBeInTheDocument();
  });

  it('offers Install Anyway when it blocks, and replans with the override', async () => {
    const blocked = planWith([missing()]);
    blocked.blockingErrors = [{
      code: 'ERR_MISSING_DEPENDENCY',
      message: 'xaeros-minimap needs Fabric API, which Agora could not find for this instance. Install it separately, or use Install Anyway.',
    }];
    const replanned = planWith([missing()]);
    replanned.warnings = [{ code: 'WARN_MISSING_DEPENDENCY', message: 'Fabric API is missing. Installing anyway, as you chose.' }];
    installFlowMocks.resolveInstallPlan.mockResolvedValueOnce(blocked).mockResolvedValueOnce(replanned);

    render(<InstallFlow intent={intent} instanceName="Test Instance" open onClose={vi.fn()} />);
    const review = await waitFor(() => {
      const found = panel();
      expect(found).toHaveAttribute('aria-modal', 'true');
      return found;
    });
    expect(within(review).getByText(/needs Fabric API/)).toBeInTheDocument();

    fireEvent.click(within(review).getByRole('button', { name: 'Install Anyway' }));

    await waitFor(() => expect(installFlowMocks.resolveInstallPlan).toHaveBeenCalledTimes(2));
    const secondIntent = installFlowMocks.resolveInstallPlan.mock.calls[1][0] as InstallIntent;
    expect(secondIntent.overrides.allowMissingDependencies).toBe(true);
    await waitFor(() => expect(within(panel()).getByText(/Fabric API is missing/)).toBeInTheDocument());
  });

  it('does not offer Install Anyway for an ordinary blocking error', async () => {
    const blocked = planWith([]);
    blocked.blockingErrors = [{ code: 'ERR_INSTANCE_LOCKED', message: 'This instance is locked.' }];
    installFlowMocks.resolveInstallPlan.mockResolvedValueOnce(blocked);
    const review = await renderReview([]);
    expect(within(review).queryByRole('button', { name: 'Install Anyway' })).toBeNull();
  });
});

describe('install review: a jar that cannot be read', () => {
  it('shows what could not be checked and offers Install Anyway', async () => {
    const unchecked = dep('xaeros-minimap', { type: 'unchecked', reason: 'the jar is not a valid archive' }, {
      source: 'jar',
      displayName: "Xaero's Minimap",
      requestedBy: ["Xaero's Minimap"],
    });
    const blocked = planWith([unchecked]);
    blocked.blockingErrors = [{
      code: 'ERR_UNCHECKED_REQUIREMENTS',
      message: "Couldn't check what Xaero's Minimap needs (the jar is not a valid archive). Install Anyway continues without that check.",
    }];
    installFlowMocks.resolveInstallPlan.mockResolvedValueOnce(blocked);
    const review = await renderReview([unchecked]);
    expect(within(review).getByText(/Couldn't check what this mod needs/)).toBeInTheDocument();
    expect(within(review).getByRole('button', { name: 'Install Anyway' })).toBeInTheDocument();
  });
});
