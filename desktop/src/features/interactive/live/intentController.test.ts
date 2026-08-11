import { describe, expect, it } from 'vitest';
import { bridgeForIntent, operationSeamFor, routeLiveIntent } from './intentController';
import { liveHighInteractionCapabilities } from './liveCapabilities';
import { liveSource } from './freshness';
import type { CapabilityFlags, VisualScene } from '../domain/models';

function freshScene(overrides: Partial<VisualScene> = {}): VisualScene {
  return {
    source: liveSource('r1', 'fresh'),
    content: [],
    relationships: [],
    findings: [],
    proposals: [],
    ...overrides,
  };
}

/** Default shipped capabilities (SOL-2 APPROVED §20): approved seams on, rejected off. */
const caps = liveHighInteractionCapabilities();

/** Test-only superset adding the still-blocked install/update seams for routing tests. */
function reviewCaps(): CapabilityFlags {
  return {
    ...caps,
    canProposeInstall: true,
    canProposeUpdate: true,
  };
}

describe('live intent controller (SOL-2 BLOCKER 2)', () => {
  it('routes selection and navigation directly', () => {
    expect(routeLiveIntent(freshScene(), { kind: 'select', entityId: 'x' }, caps, 'inst-1')).toEqual({ status: 'selection' });
    expect(routeLiveIntent(freshScene(), { kind: 'open-guide', topicId: 'instances' }, caps, 'inst-1')).toEqual({
      status: 'navigate',
      intent: { kind: 'open-guide', topicId: 'instances' },
    });
  });

  it('routes approved review intents to their bridge with context', () => {
    const loader = routeLiveIntent(freshScene(), { kind: 'review-loader' }, reviewCaps(), 'inst-1');
    expect(loader).toMatchObject({ status: 'review', route: { bridge: 'loader-review' } });
    if (loader.status === 'review') expect(loader.route.context).toMatchObject({ kind: 'loader-review', instanceId: 'inst-1' });

    const health = routeLiveIntent(freshScene(), { kind: 'review-health' }, reviewCaps(), 'inst-1');
    expect(health).toMatchObject({ status: 'review', route: { bridge: 'health-review' } });
    if (health.status === 'review') expect(health.route.context).toMatchObject({ kind: 'health-review', instanceId: 'inst-1' });

    const crash = routeLiveIntent(freshScene(), { kind: 'open-crash-doctor' }, reviewCaps(), 'inst-1');
    expect(crash).toMatchObject({ status: 'review', route: { bridge: 'crash-doctor' } });
    if (crash.status === 'review') expect(crash.route.context).toMatchObject({ kind: 'crash-doctor', instanceId: 'inst-1' });

    const snap = routeLiveIntent(freshScene(), { kind: 'preview-snapshot', snapshotId: 's1' }, reviewCaps(), 'inst-1');
    expect(snap).toMatchObject({ status: 'review', route: { bridge: 'snapshot-compare' } });
    if (snap.status === 'review') expect(snap.route.context).toMatchObject({ kind: 'snapshot-compare', instanceId: 'inst-1', snapshotId: 's1' });

    const staged = routeLiveIntent(freshScene(), { kind: 'review-staged-changes' }, reviewCaps(), 'inst-1');
    expect(staged).toMatchObject({ status: 'review', route: { bridge: 'install-flow' } });
    if (staged.status === 'review') expect(staged.route.context).toMatchObject({ kind: 'install-flow', instanceId: 'inst-1', action: 'review' });
  });

  it('retains the requested action + content on install-flow routes (never inferred from an id)', () => {
    const install = routeLiveIntent(freshScene(), { kind: 'propose-install', contentId: 'm1' }, reviewCaps(), 'inst-1');
    expect(install).toMatchObject({ status: 'review', route: { bridge: 'install-flow' } });
    if (install.status === 'review') expect(install.route.context).toMatchObject({ kind: 'install-flow', action: 'install', contentId: 'm1' });

    const update = routeLiveIntent(freshScene(), { kind: 'propose-update', contentId: 'm1' }, reviewCaps(), 'inst-1');
    expect(update).toMatchObject({ status: 'review', route: { bridge: 'install-flow' } });
    if (update.status === 'review') expect(update.route.context).toMatchObject({ kind: 'install-flow', action: 'update', contentId: 'm1' });

    const remove = routeLiveIntent(freshScene(), { kind: 'propose-remove', contentId: 'm1' }, reviewCaps(), 'inst-1');
    expect(remove).toMatchObject({ status: 'review', route: { bridge: 'install-flow' } });
    if (remove.status === 'review') expect(remove.route.context).toMatchObject({ kind: 'install-flow', action: 'remove', contentId: 'm1' });
  });

  it('shipped capabilities enable the SOL-2-approved seams and route them to review', () => {
    const cases: Array<{ intent: Parameters<typeof routeLiveIntent>[1]; bridge: string }> = [
      { intent: { kind: 'review-loader' }, bridge: 'loader-review' },
      { intent: { kind: 'review-health' }, bridge: 'health-review' },
      { intent: { kind: 'open-crash-doctor' }, bridge: 'crash-doctor' },
      { intent: { kind: 'preview-snapshot', snapshotId: 's1' }, bridge: 'snapshot-compare' },
      { intent: { kind: 'propose-remove', contentId: 'm1' }, bridge: 'install-flow' },
    ];
    for (const c of cases) {
      const result = routeLiveIntent(freshScene(), c.intent, caps, 'inst-1');
      expect(result.status).toBe('review');
      if (result.status === 'review') expect(result.route.bridge).toBe(c.bridge);
    }
  });

  it('shipped capabilities keep rejected/blocked seams OFF (install/update, disable, restore, memory, offline)', () => {
    const intents: Parameters<typeof routeLiveIntent>[1][] = [
      { kind: 'review-staged-changes' },
      { kind: 'propose-install', contentId: 'm1' },
      { kind: 'propose-update', contentId: 'm1' },
      { kind: 'propose-enabled', contentId: 'm1', enabled: false },
      { kind: 'request-snapshot-restore', snapshotId: 's1' },
      { kind: 'propose-memory', mode: 'manual', memoryMiB: 4096 },
      { kind: 'review-offline-readiness' },
    ];
    for (const intent of intents) {
      const result = routeLiveIntent(freshScene(), intent, caps, 'inst-1');
      expect(result.status).toBe('blocked');
      if (result.status === 'blocked') expect(result.gate).toBe('capability');
    }
  });

  it('blocks rejected seams (disable, restore, memory) by capability even when approved caps are on', () => {
    const blocked = routeLiveIntent(freshScene(), { kind: 'request-snapshot-restore', snapshotId: 's1' }, reviewCaps(), 'inst-1');
    expect(blocked.status).toBe('blocked');
    if (blocked.status === 'blocked') expect(blocked.gate).toBe('capability');
    expect(routeLiveIntent(freshScene(), { kind: 'propose-enabled', contentId: 'm1', enabled: false }, reviewCaps(), 'inst-1').status).toBe('blocked');
    expect(routeLiveIntent(freshScene(), { kind: 'propose-memory', mode: 'manual', memoryMiB: 4096 }, reviewCaps(), 'inst-1').status).toBe('blocked');
  });

  it('requires a fresh scene before any review (non-fresh is never executable)', () => {
    const stale = freshScene({ source: liveSource('r1', 'stale') });
    expect(routeLiveIntent(stale, { kind: 'review-loader' }, reviewCaps(), 'inst-1')).toEqual({ status: 'refresh-required' });
    const refreshing = freshScene({ source: liveSource('r1', 'refreshing') });
    expect(routeLiveIntent(refreshing, { kind: 'review-loader' }, reviewCaps(), 'inst-1')).toEqual({ status: 'refresh-required' });
    const unknown = freshScene({ source: liveSource('r1', 'unknown') });
    expect(routeLiveIntent(unknown, { kind: 'review-loader' }, reviewCaps(), 'inst-1')).toEqual({ status: 'refresh-required' });
  });

  it('missing scene blocks with missing-source', () => {
    const result = routeLiveIntent(undefined, { kind: 'review-loader' }, reviewCaps(), 'inst-1');
    expect(result.status).toBe('blocked');
    if (result.status === 'blocked') expect(result.gate).toBe('missing-source');
  });

  it('blocks a review while a canonical operation is busy (availability gate)', () => {
    const result = routeLiveIntent(
      freshScene(),
      { kind: 'review-health' },
      reviewCaps(),
      'inst-1',
      { locked: false, recoveryBusy: false, processBusy: false, installBusy: true },
    );
    expect(result.status).toBe('blocked');
    if (result.status === 'blocked') {
      expect(result.gate).toBe('availability');
      expect(result.reason).toMatch(/install is active/i);
    }
  });

  it('blocks review for each unavailable readiness/lock state with its own explanation (SOL-2 §18.3)', () => {
    const cases: Array<{ availability: Parameters<typeof routeLiveIntent>[4]; reason: RegExp }> = [
      { availability: { locked: true, recoveryBusy: false, processBusy: false, installBusy: false }, reason: /locked by another player/i },
      { availability: { locked: false, recoveryBusy: true, processBusy: false, installBusy: false }, reason: /recovery is pending or failed/i },
      { availability: { locked: false, recoveryBusy: false, processBusy: true, installBusy: false }, reason: /process is active/i },
      { availability: { locked: false, recoveryBusy: false, processBusy: false, installBusy: true }, reason: /install is active/i },
    ];
    for (const c of cases) {
      const result = routeLiveIntent(freshScene(), { kind: 'review-loader' }, reviewCaps(), 'inst-1', c.availability);
      expect(result.status).toBe('blocked');
      if (result.status === 'blocked') {
        expect(result.gate).toBe('availability');
        expect(result.reason).toMatch(c.reason);
      }
    }
    // Selection and inspection stay available regardless of readiness/locks.
    expect(routeLiveIntent(freshScene(), { kind: 'select', entityId: 'x' }, reviewCaps(), 'inst-1', {
      locked: true,
      recoveryBusy: true,
      processBusy: true,
      installBusy: true,
    })).toEqual({ status: 'selection' });
  });

  it('blocks a second review while one is in flight (coalesces duplicates)', () => {
    const inFlight = freshScene({
      proposals: [
        {
          id: 'p1',
          intent: { kind: 'propose-install', contentId: 'm1' },
          phase: 'in-review',
          title: 'Install',
          summary: '',
          destructive: false,
        },
      ],
    });
    const result = routeLiveIntent(inFlight, { kind: 'review-health' }, reviewCaps(), 'inst-1');
    expect(result.status).toBe('blocked');
    if (result.status === 'blocked') expect(result.gate).toBe('availability');
  });

  it('maps bridges to the recorded authoritative seams (never invoked here)', () => {
    expect(bridgeForIntent({ kind: 'review-staged-changes' })).toBe('install-flow');
    expect(operationSeamFor('crash-doctor')).toContain('CrashInvestigator');
    expect(operationSeamFor('install-flow')).toContain('InstallFlow');
    expect(bridgeForIntent({ kind: 'request-snapshot-restore', snapshotId: 's1' })).toBeNull();
  });
});
