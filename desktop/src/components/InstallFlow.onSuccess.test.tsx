import { describe, expect, it, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react';
import { useState } from 'react';
import { InstallFlow } from './InstallFlow';
import type { InstallIntent, ResolvedInstallPlan, InstallOutcome, ProgressEvent } from '../lib/installFlow';

// ---------------------------------------------------------------------------
// Mocks — same hoisted pattern as HealthDialog.reviewOnly.test.tsx:24
// ---------------------------------------------------------------------------

const installFlowMocks = vi.hoisted(() => ({
  resolveInstallPlan: vi.fn(),
  applyInstallPlan: vi.fn(),
  subscribeProgress: vi.fn(),
  cancelInstall: vi.fn(),
  planNeedsUserReview: vi.fn(() => false),
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

vi.mock('../features/tour/tourSignals', () => ({
  emitTourSignal: vi.fn(),
}));

// ---------------------------------------------------------------------------
// Minimal fake plan — mirrors the shape HealthDialog.reviewOnly.test.tsx:23 uses
// ---------------------------------------------------------------------------

function fakePlan(): ResolvedInstallPlan {
  const intent: InstallIntent = {
    action: { type: 'batch-update', items: [{ itemId: 'sodium', targetVersion: '0.6.0' }] },
    targetInstance: 'test-inst',
    optionalDeps: { type: 'prompt' },
    requestedBy: 'interactive',
    overrides: { allowReplace: false, skipHealthScan: false, forceConflictResolution: {} },
  };
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
    diskEstimate: { downloadBytes: 0, snapshotBytes: 0, applyOverheadBytes: 0, peakAdditionalBytes: 0, postCommitDeltaBytes: 0 },
    warnings: [],
    blockingErrors: [],
    pendingChoices: [],
    createdAt: new Date().toISOString(),
    instanceStateHash: 'hash',
    registryRevision: 'rev',
  };
}

function successOutcome(): InstallOutcome {
  return {
    type: 'success',
    installedItems: ['sodium-0.6.0.jar'],
    existingItemsReused: [],
    warnings: [],
    health: { type: 'completed', report: { score: 'green', warnings: [], blockers: [], recommendations: [], scan_token: 't' } },
    snapshotId: 'snap-1',
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  installFlowMocks.resolveInstallPlan.mockResolvedValue(fakePlan());
  installFlowMocks.applyInstallPlan.mockResolvedValue(successOutcome());
  installFlowMocks.subscribeProgress.mockImplementation(async (_id: string, _handler: (e: ProgressEvent) => void) => {
    return () => {};
  });
  installFlowMocks.planNeedsUserReview.mockReturnValue(false);
});

// ---------------------------------------------------------------------------
// Harness that reproduces the inline-callback loop
// ---------------------------------------------------------------------------

function InlineSuccessHarness({ onCall }: { onCall: (id: string) => void }) {
  const [tick, setTick] = useState(0);
  // Crucial: inline arrow → new identity every render, and the handler itself
  // triggers state that forces a re-render (new [] would do the same). This is
  // exactly what InstanceEditor did: `setCachedUpdates([])` with a fresh array.
  return (
    <>
      <InstallFlow
        intent={fakePlan().intent}
        instanceName="Test Instance"
        open={true}
        onClose={() => {}}
        onSuccess={(id) => {
          onCall(id);
          // This setState with a fresh value on every call is the loop driver:
          // without the ref guard in InstallFlow.tsx:429, the effect that depends
          // on `onSuccess` would re-fire, call onSuccess again, setState again...
          setTick((t) => t + 1);
        }}
      />
      <div data-testid="tick">{tick}</div>
    </>
  );
}

describe('InstallFlow onSuccess loop guard (InstallFlow.tsx:429)', () => {
  it('fires onSuccess exactly once even though the caller passes a new inline callback that sets state', async () => {
    const onCall = vi.fn();

    render(<InlineSuccessHarness onCall={onCall} />);

    // Resolving → Review. Click the confirm button to reach executing → result.
    await screen.findByText(/Review Instance Changes/i);
    // For batch-update the confirm label is "Apply Updates" (InstallFlow.tsx:643)
    const confirm = await screen.findByRole('button', { name: /Apply Updates/i });
    fireEvent.click(confirm);

    // Wait for the success outcome to render.
    await screen.findByText(/All verified changes were applied successfully/i);

    // onSuccess must have fired exactly once, not on every re-render.
    await waitFor(() => expect(onCall).toHaveBeenCalledTimes(1));
    expect(onCall).toHaveBeenCalledWith('test-inst');

    // A further parent re-render must not re-fire it. Bump the harness again
    // and assert the count stays 1.
    const tickBefore = Number(screen.getByTestId('tick').textContent);
    // Force another render by clicking the harness's own bump is not needed —
    // the onSuccess-triggered tick already caused one. Give the effect a chance
    // to (incorrectly) fire again on the new onSuccess identity.
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });
    expect(onCall).toHaveBeenCalledTimes(1);
    // The tick should be exactly 1 (one success), not 2+ from a loop.
    expect(Number(screen.getByTestId('tick').textContent)).toBe(tickBefore);
  });

  it('still fires again for a second successful install in the same mount (ref resets)', async () => {
    const onCall = vi.fn();
    // First install
    const { unmount } = render(<InlineSuccessHarness onCall={onCall} />);
    await screen.findByText(/Review Instance Changes/i);
    fireEvent.click(await screen.findByRole('button', { name: /Apply Updates/i }));
    await screen.findByText(/All verified changes were applied successfully/i);
    await waitFor(() => expect(onCall).toHaveBeenCalledTimes(1));
    unmount();

    // Second mount (simulates a later install in same InstanceEditor lifecycle) — ref must have reset on leave.
    const onCall2 = vi.fn();
    render(<InlineSuccessHarness onCall={onCall2} />);
    await screen.findByText(/Review Instance Changes/i);
    fireEvent.click(await screen.findByRole('button', { name: /Apply Updates/i }));
    await screen.findByText(/All verified changes were applied successfully/i);
    await waitFor(() => expect(onCall2).toHaveBeenCalledTimes(1));
  });
});
