import { describe, expect, it, vi } from 'vitest';
import { openBridge, installActionLabel } from './operationBridges';
import type { LiveReviewRoute, StandardBridgeHandlers } from './operationBridges';

function handlers() {
  return {
    openHealthReview: vi.fn(),
    openLoaderReview: vi.fn(),
    openSnapshotCompare: vi.fn(),
    openCrashDoctor: vi.fn(),
    openInstallFlow: vi.fn(),
  } satisfies StandardBridgeHandlers;
}

describe('operation bridges (SOL-2 §17.4: every dormant adapter)', () => {
  it('health-review dispatches to the fresh HealthDialog handler with instance identity', () => {
    const h = handlers();
    const route: LiveReviewRoute = { bridge: 'health-review', context: { kind: 'health-review', instanceId: 'inst-1' } };
    expect(openBridge(route, h)).toBe(true);
    expect(h.openHealthReview).toHaveBeenCalledWith({ instanceId: 'inst-1', instanceName: 'inst-1' });
  });

  it('loader-review dispatches to the LoaderChooser handler with instance identity', () => {
    const h = handlers();
    const route: LiveReviewRoute = { bridge: 'loader-review', context: { kind: 'loader-review', instanceId: 'inst-1' } };
    expect(openBridge(route, h)).toBe(true);
    expect(h.openLoaderReview).toHaveBeenCalledWith({ instanceId: 'inst-1' });
  });

  it('snapshot-compare dispatches to the selected detectDrift handler', () => {
    const h = handlers();
    const route: LiveReviewRoute = { bridge: 'snapshot-compare', context: { kind: 'snapshot-compare', instanceId: 'inst-1', snapshotId: 's1' } };
    expect(openBridge(route, h)).toBe(true);
    expect(h.openSnapshotCompare).toHaveBeenCalledWith({ instanceId: 'inst-1', snapshotId: 's1' });
  });

  it('crash-doctor dispatches to the navigation-only CrashInvestigator handler', () => {
    const h = handlers();
    const route: LiveReviewRoute = { bridge: 'crash-doctor', context: { kind: 'crash-doctor', instanceId: 'inst-1' } };
    expect(openBridge(route, h)).toBe(true);
    expect(h.openCrashDoctor).toHaveBeenCalledWith({ instanceId: 'inst-1' });
  });

  it('install-flow passes the retained action + content to the InstallFlow handler', () => {
    const h = handlers();
    const route: LiveReviewRoute = {
      bridge: 'install-flow',
      context: { kind: 'install-flow', instanceId: 'inst-1', action: 'remove', contentId: 'live:content:mod:sodium.jar', filename: 'sodium.jar', contentKind: 'mod' },
    };
    expect(openBridge(route, h)).toBe(true);
    expect(h.openInstallFlow).toHaveBeenCalledWith({
      instanceId: 'inst-1',
      action: 'remove',
      contentId: 'live:content:mod:sodium.jar',
      filename: 'sodium.jar',
    });
  });

  it('install-flow without resolved content omits optional fields (never invents identity)', () => {
    const h = handlers();
    const route: LiveReviewRoute = {
      bridge: 'install-flow',
      context: { kind: 'install-flow', instanceId: 'inst-1', action: 'review' },
    };
    expect(openBridge(route, h)).toBe(true);
    expect(h.openInstallFlow).toHaveBeenCalledWith({ instanceId: 'inst-1', action: 'review' });
  });

  it('labels install-flow actions for the review proposal', () => {
    expect(installActionLabel('install')).toBe('Install');
    expect(installActionLabel('update')).toBe('Update');
    expect(installActionLabel('remove')).toBe('Remove');
    expect(installActionLabel('review')).toBe('Review staged changes');
  });
});
