/**
 * Where the install flow renders depends on whether it is asking anything.
 *
 * The review panel used to live in the corner permanently. With a mouse that
 * was merely small; with a controller it was one tiny target among everything
 * else on the page, and reaching it was the complaint that prompted this. It
 * now takes the screen while it needs a decision — and, just as importantly,
 * does *not* while an install is simply running, because stealing focus for
 * background progress interrupts whatever the user was actually doing.
 */
import { describe, expect, it, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { InstallFlow } from './InstallFlow';
import type { InstallIntent, ResolvedInstallPlan, ProgressEvent } from '../lib/installFlow';

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
    listSnapshots: vi.fn(async () => []),
    getCachedInstanceUpdates: vi.fn(async () => []),
    clearCachedInstanceUpdates: vi.fn(async () => {}),
  };
});

vi.mock('../features/tour/tourSignals', () => ({ emitTourSignal: vi.fn() }));

const intent: InstallIntent = {
  action: { type: 'batch-update', items: [{ itemId: 'sodium', targetVersion: '0.6.0' }] },
  targetInstance: 'test-inst',
  optionalDeps: { type: 'prompt' },
  requestedBy: 'interactive',
  overrides: { allowReplace: false, skipHealthScan: false, forceConflictResolution: {} },
};

function fakePlan(): ResolvedInstallPlan {
  return {
    fingerprint: 'fp-test-123',
    intent,
    operation: { type: 'batch-update', operations: [] },
    dependencies: [],
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

function successOutcome() {
  return {
    type: 'success' as const,
    installedItems: [],
    existingItemsReused: [],
    warnings: [],
    health: { type: 'completed' as const, report: {} },
    snapshotId: 'snap-presentation',
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  installFlowMocks.resolveInstallPlan.mockResolvedValue(fakePlan());
  installFlowMocks.subscribeProgress.mockImplementation(
    async (_id: string, _handler: (e: ProgressEvent) => void) => () => {},
  );
  installFlowMocks.planNeedsUserReview.mockReturnValue(true);
});

const panel = () => screen.getByRole('dialog', { name: /Review Instance Changes/i });

function renderFlow() {
  return renderFlowWith(vi.fn());
}

function renderFlowWith(onClose: () => void) {
  return render(
    <InstallFlow intent={intent} instanceName="Test Instance" open onClose={onClose} />,
  );
}

describe('install flow presentation', () => {
  it('takes the screen while it is waiting for a review decision', async () => {
    renderFlow();

    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'true'));
    expect(panel().className).not.toContain('bottom-4');
  });

  it('stays out of the corner-panel role while modal', async () => {
    renderFlow();

    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'true'));
    // A modal is not a live region: it is the task, not an announcement.
    expect(panel()).not.toHaveAttribute('aria-live');
  });

  it('sits in the corner and stays non-modal while resolving', async () => {
    // Never resolves, so the flow stays in its progress phase.
    installFlowMocks.resolveInstallPlan.mockImplementation(() => new Promise(() => {}));
    renderFlow();

    await waitFor(() => expect(panel()).toBeInTheDocument());
    expect(panel()).toHaveAttribute('aria-modal', 'false');
    expect(panel().className).toContain('bottom-4');
    expect(panel()).toHaveAttribute('aria-live', 'polite');
  });

  it('Escape closes the corner panel while focus is inside it', async () => {
    installFlowMocks.resolveInstallPlan.mockImplementation(() => new Promise(() => {}));
    const onClose = vi.fn();
    render(<InstallFlow intent={intent} instanceName="Test Instance" open onClose={onClose} />);
    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'false'));

    const close = within(panel()).getByRole('button', { name: 'Close install panel' });
    close.focus();
    await userEvent.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('Escape does not cancel an install that is running', async () => {
    installFlowMocks.applyInstallPlan.mockImplementation(() => new Promise(() => {}));
    const onClose = vi.fn();
    renderFlowWith(onClose);
    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'true'));
    await userEvent.click(within(panel()).getByRole('button', { name: 'Apply Updates' }));
    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'false'));

    within(panel()).getByRole('button', { name: 'Close install panel' }).focus();
    await userEvent.keyboard('{Escape}');
    expect(installFlowMocks.cancelInstall).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('Escape does nothing while focus is outside the corner panel', async () => {
    installFlowMocks.resolveInstallPlan.mockImplementation(() => new Promise(() => {}));
    const onClose = vi.fn();
    render(<InstallFlow intent={intent} instanceName="Test Instance" open onClose={onClose} />);
    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'false'));

    document.body.focus();
    await userEvent.keyboard('{Escape}');
    expect(onClose).not.toHaveBeenCalled();
  });

  it('the header Close dismisses a finished install result', async () => {
    installFlowMocks.applyInstallPlan.mockResolvedValue(successOutcome());
    const onClose = vi.fn();
    render(<InstallFlow intent={intent} instanceName="Test Instance" open onClose={onClose} />);
    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'true'));
    await userEvent.click(within(panel()).getByRole('button', { name: 'Apply Updates' }));

    await waitFor(() => expect(within(panel()).getByRole('button', { name: 'Open Instance' })).toBeInTheDocument());
    await userEvent.click(within(panel()).getByRole('button', { name: 'Close install panel' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('when the install lands with focus in the panel, focus moves to Open Instance', async () => {
    let finish: (outcome: unknown) => void = () => {};
    installFlowMocks.applyInstallPlan.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    renderFlow();
    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'true'));
    await userEvent.click(within(panel()).getByRole('button', { name: 'Apply Updates' }));

    // Running: the corner panel, with focus inside it.
    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'false'));
    within(panel()).getByRole('button', { name: 'Close install panel' }).focus();
    finish(successOutcome());

    await waitFor(() => expect(within(panel()).getByRole('button', { name: 'Open Instance' })).toHaveFocus());
    expect(panel()).toHaveAttribute('aria-modal', 'true');
  });

  it('when the install lands with focus outside the panel, focus is not pulled to Open Instance', async () => {
    let finish: (outcome: unknown) => void = () => {};
    installFlowMocks.applyInstallPlan.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    renderFlow();
    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'true'));
    await userEvent.click(within(panel()).getByRole('button', { name: 'Apply Updates' }));
    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'false'));
    document.body.focus();
    finish(successOutcome());

    await waitFor(() => expect(within(panel()).getByRole('button', { name: 'Open Instance' })).toBeInTheDocument());
    expect(within(panel()).getByRole('button', { name: 'Open Instance' })).not.toHaveFocus();
  });

  it('keeps the same tour anchor in both presentations', async () => {
    installFlowMocks.resolveInstallPlan.mockImplementation(() => new Promise(() => {}));
    const { unmount } = renderFlow();
    await waitFor(() => expect(panel()).toHaveAttribute('data-tour', 'install-review-dialog'));
    unmount();

    installFlowMocks.resolveInstallPlan.mockResolvedValue(fakePlan());
    renderFlow();
    await waitFor(() => expect(panel()).toHaveAttribute('aria-modal', 'true'));
    expect(panel()).toHaveAttribute('data-tour', 'install-review-dialog');
  });
});
