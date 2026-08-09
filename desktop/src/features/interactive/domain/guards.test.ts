import { describe, expect, it } from 'vitest';
import { capabilityGate, freshnessGate, gateLiveIntent, sourceGate } from './guards';
import type { CapabilityFlags, ExperienceSource, VisualScene } from './models';
import { NO_CAPABILITIES } from './models';

const liveSource: ExperienceSource = {
  kind: 'live',
  viewRevision: 'v1',
  observedAt: '2026-01-01T00:00:00Z',
  freshness: 'fresh',
};

const liveScene: VisualScene = {
  source: liveSource,
  content: [],
  relationships: [],
  findings: [],
  proposals: [],
};

function capabilities(partial: Partial<CapabilityFlags>): CapabilityFlags {
  return { ...NO_CAPABILITIES, ...partial };
}

describe('domain/guards', () => {
  it('source gate rejects simulation scenes for live intents', () => {
    expect(sourceGate({ kind: 'simulation', scenarioId: 'x', scenarioVersion: 1 })).toEqual({
      ok: false,
      reason: 'simulation-source',
    });
    expect(sourceGate(liveSource)).toEqual({ ok: true });
    expect(sourceGate(undefined)).toEqual({ ok: false, reason: 'missing-source' });
  });

  it('capability gate blocks actions without an approved bridge', () => {
    const capped = capabilities({ canReviewLoader: true });
    expect(capabilityGate({ kind: 'review-loader' }, capped)).toEqual({ ok: true });
    expect(capabilityGate({ kind: 'request-snapshot-restore', snapshotId: 's1' }, capped)).toEqual({
      ok: false,
      reason: 'capability',
    });
    // exploration/navigation is always available
    expect(capabilityGate({ kind: 'select', entityId: 'x' }, NO_CAPABILITIES)).toEqual({ ok: true });
  });

  it('freshness gate rejects stale or unknown scenes', () => {
    expect(freshnessGate(liveSource)).toEqual({ ok: true });
    expect(freshnessGate({ ...liveSource, freshness: 'stale' })).toEqual({ ok: false, reason: 'stale' });
    expect(freshnessGate({ ...liveSource, freshness: 'unknown' })).toEqual({ ok: false, reason: 'unknown' });
    // simulation scenes are reducer-controlled
    expect(freshnessGate({ kind: 'simulation', scenarioId: 'x', scenarioVersion: 1 })).toEqual({ ok: true });
  });

  it('combined gate requires source, capability, and freshness in order', () => {
    expect(gateLiveIntent(liveScene, { kind: 'review-loader' }, capabilities({ canReviewLoader: true }))).toEqual({
      ok: true,
    });
    // stale beats capability ordering: source ok, capability ok, freshness fails
    expect(
      gateLiveIntent(
        { ...liveScene, source: { ...liveSource, freshness: 'stale' } },
        { kind: 'review-loader' },
        capabilities({ canReviewLoader: true }),
      ),
    ).toEqual({ ok: false, reason: 'stale' });
    // missing scene
    expect(gateLiveIntent(undefined, { kind: 'review-loader' }, capabilities({ canReviewLoader: true }))).toEqual({
      ok: false,
      reason: 'missing-source',
    });
  });
});
