import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { LiveInteractiveHost } from './LiveInteractiveHost';
import { ok, err } from './liveScene';
import type { CapabilityFlags, VisualContentNode, VisualScene } from '../domain/models';
import type { LiveHostData } from './LiveSceneView';
import { liveSource } from './freshness';

function contentNode(overrides: Partial<VisualContentNode> = {}): VisualContentNode {
  return {
    id: 'live:content:mod:sodium.jar',
    name: 'sodium.jar',
    kind: 'mod',
    presence: { current: 'not-installed' },
    enabled: { current: false },
    health: 'unknown',
    relationshipSummary: { requiredBy: 0, requires: 0, conflicts: 0 },
    availability: 'available',
    ...overrides,
  };
}

function baseScene(): VisualScene {
  return {
    source: liveSource('r0', 'fresh'),
    instance: {
      id: 'inst-1',
      name: 'My World',
      gameVersion: '1.20.1',
      loader: { current: { family: 'Fabric', compatibility: 'unknown' } },
      lockState: 'editable',
      recoveryReadiness: 'ready',
      launchState: 'idle',
      contentSummary: { enabled: 1, disabled: 0, needsAttention: 0 },
    },
    content: [],
    relationships: [],
    findings: [],
    proposals: [],
  };
}

function makeData(overrides: Partial<VisualScene> = {}, health: 'ok' | 'err' = 'ok'): LiveHostData {
  return {
    scene: { ...baseScene(), ...overrides },
    health: health === 'ok' ? ok(true) : err<boolean>(),
    snapshots: ok([]),
    crashEvidence: ok(null),
    runtime: ok(null),
  };
}

/** Test-only caps enabling the approved review bridges (default stays all-false). */
function reviewCaps(): CapabilityFlags {
  return {
    canProposeInstall: true,
    canProposeUpdate: false,
    canProposeRemove: false,
    canProposeEnabled: false,
    canReviewHealth: true,
    canReviewLoader: true,
    canOpenCrashDoctor: true,
    canPreviewSnapshot: true,
    canRequestSnapshotRestore: false,
    canProposeMemory: false,
    canReviewOfflineReadiness: false,
  };
}

describe('LiveInteractiveHost (High Interaction live surface)', () => {
  it('renders the scene after loading with the Standard escape', async () => {
    const load = vi.fn(async (): Promise<LiveHostData> => makeData());
    render(<LiveInteractiveHost instanceId="inst-1" onUseStandardView={() => undefined} load={load} />);
    expect(screen.getByText('Loading live data…')).toBeInTheDocument();
    await waitFor(() => expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1));
    // No "High Interaction" heading: the surface announces itself, and the
    // control row carries Back / Refresh / Standard on ONE line instead.
    expect(screen.queryByRole('heading', { name: 'High Interaction' })).toBeNull();
    expect(screen.getByRole('button', { name: /Use Standard view/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeInTheDocument();
  });

  it('shows a fail-closed error state with Retry and Standard escape on adapter failure', async () => {
    const load = vi.fn(async (): Promise<LiveHostData> => {
      throw new Error('backend offline');
    });
    render(<LiveInteractiveHost instanceId="inst-1" onUseStandardView={() => undefined} load={load} />);
    await waitFor(() => expect(screen.getByText(/could not load live data/i)).toBeInTheDocument());
    expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: /Use Standard view/ }).length).toBeGreaterThanOrEqual(1);
  });

  it('shows an error when the scene has no instance (never a valid empty instance)', async () => {
    const load = vi.fn(async (): Promise<LiveHostData> => makeData({ instance: undefined }));
    render(<LiveInteractiveHost instanceId="inst-1" onUseStandardView={() => undefined} load={load} />);
    await waitFor(() => expect(screen.getByText(/no readable state/i)).toBeInTheDocument());
  });

  it('renders health as unavailable (not ready) when the health read failed', async () => {
    const load = vi.fn(async (): Promise<LiveHostData> => makeData({}, 'err'));
    render(<LiveInteractiveHost instanceId="inst-1" onUseStandardView={() => undefined} load={load} />);
    await waitFor(() => expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1));
    expect(screen.getByText(/health could not be verified/i)).toBeInTheDocument();
    expect(screen.queryByText(/ready to launch/i)).not.toBeInTheDocument();
  });

  it('refresh keeps the last scene visible (latest-wins) and re-reads with a new revision', async () => {
    let calls = 0;
    const revisions: string[] = [];
    const load = vi.fn(async (_id: string, revision: string): Promise<LiveHostData> => {
      calls += 1;
      revisions.push(revision);
      return makeData();
    });
    render(<LiveInteractiveHost instanceId="inst-1" onUseStandardView={() => undefined} load={load} />);
    await waitFor(() => expect(calls).toBe(1));
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(calls).toBe(2));
    expect(new Set(revisions).size).toBe(2);
    // The scene stayed visible (not a loading-only screen) across the refresh.
    expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1);
  });

  it('marks the retained scene refreshing (non-executable) while a refresh is in flight', async () => {
    let resolveRefresh: (d: LiveHostData) => void = () => undefined;
    let calls = 0;
    const load = vi.fn((): Promise<LiveHostData> => {
      calls += 1;
      if (calls === 1) return Promise.resolve(makeData());
      return new Promise((resolve) => { resolveRefresh = resolve; });
    });
    render(<LiveInteractiveHost instanceId="inst-1" onUseStandardView={() => undefined} load={load} />);
    await waitFor(() => expect(calls).toBe(1));
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    // While the refresh is unresolved the retained scene is visibly refreshing.
    await waitFor(() => expect(screen.getAllByText(/refreshing/i).length).toBeGreaterThanOrEqual(1));
    expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1);
    resolveRefresh(makeData());
    await waitFor(() => expect(screen.queryByText(/refreshing/i)).not.toBeInTheDocument());
  });

  it('discards an out-of-order older refresh result (latest wins)', async () => {
    let resolveFirst: (d: LiveHostData) => void = () => undefined;
    let resolveSecond: (d: LiveHostData) => void = () => undefined;
    let calls = 0;
    const load = vi.fn((): Promise<LiveHostData> => {
      calls += 1;
      if (calls === 1) return new Promise((resolve) => { resolveFirst = resolve; });
      return new Promise((resolve) => { resolveSecond = resolve; });
    });
    render(<LiveInteractiveHost instanceId="inst-1" onUseStandardView={() => undefined} load={load} />);
    await waitFor(() => expect(calls).toBe(1));
    resolveFirst(makeData());
    await waitFor(() => expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1));

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(calls).toBe(2));
    // Resolve the OLD (first) request late with a spoofed name: it must be
    // discarded because a newer generation is in flight.
    resolveFirst(makeData({ instance: { ...baseScene().instance!, name: 'OLD RESULT' } }));
    resolveSecond(makeData());
    await waitFor(() => expect(screen.queryByText('OLD RESULT')).not.toBeInTheDocument());
    expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1);
  });

  it('discards results from a previous instance after an instance switch', async () => {
    let resolveOld: (d: LiveHostData) => void = () => undefined;
    const load = vi.fn((id: string): Promise<LiveHostData> => {
      if (id === 'inst-A') {
        return new Promise((resolve) => { resolveOld = resolve; });
      }
      return Promise.resolve(makeData({ instance: { ...baseScene().instance!, id: 'inst-B', name: 'World B' } }));
    });
    const { rerender } = render(<LiveInteractiveHost instanceId="inst-A" onUseStandardView={() => undefined} load={load} />);
    await waitFor(() => expect(load).toHaveBeenCalledWith('inst-A', expect.any(String)));
    // Switch to inst-B while inst-A's read is still unresolved.
    rerender(<LiveInteractiveHost instanceId="inst-B" onUseStandardView={() => undefined} load={load} />);
    await waitFor(() => expect(screen.getAllByText('World B').length).toBeGreaterThanOrEqual(1));
    // Resolve the stale inst-A result late — it must be discarded.
    resolveOld(makeData({ instance: { ...baseScene().instance!, id: 'inst-A', name: 'OLD A' } }));
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByText('OLD A')).not.toBeInTheDocument();
    expect(screen.getAllByText('World B').length).toBeGreaterThanOrEqual(1);
  });

  it('never executes a review while the scene is refreshing (routes to refresh instead)', async () => {
    let resolveRefresh: (d: LiveHostData) => void = () => undefined;
    let calls = 0;
    const scene = baseScene();
    scene.findings = [
      {
        id: 'f1',
        severity: 'blocker',
        title: 'Loader does not fit',
        summary: 'Incompatible.',
        affectedIds: [],
        structuredKind: 'loader-compatibility',
        reviewIntent: { kind: 'review-loader' },
      },
    ];
    const load = vi.fn((): Promise<LiveHostData> => {
      calls += 1;
      if (calls === 1) return Promise.resolve(makeData(scene));
      return new Promise((resolve) => { resolveRefresh = resolve; });
    });
    const onOpenStandardOperation = vi.fn();
    render(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        onOpenStandardOperation={onOpenStandardOperation}
        load={load}
        capabilities={reviewCaps()}
      />,
    );
    await waitFor(() => expect(screen.getByText('Loader does not fit')).toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(calls).toBe(2));
    // During the in-flight refresh the Review button must not execute.
    const reviewButtons = screen.queryAllByRole('button', { name: 'Review' });
    for (const button of reviewButtons) fireEvent.click(button);
    expect(onOpenStandardOperation).not.toHaveBeenCalled();
    resolveRefresh(makeData(scene));
    await waitFor(() => expect(onOpenStandardOperation).toHaveBeenCalledTimes(0));
  });

  it('projects canonical phases: launching, running, stopping, delegated, failed', async () => {
    const cases: Array<{ phase: string; launch: string; busy: boolean }> = [
      { phase: 'launching', launch: 'starting', busy: true },
      { phase: 'running', launch: 'running', busy: true },
      { phase: 'stopping', launch: 'stopping', busy: true },
      { phase: 'delegated', launch: 'delegated', busy: true },
      { phase: 'failed', launch: 'failed', busy: false },
      { phase: 'idle', launch: 'idle', busy: false },
    ];
    for (const c of cases) {
      const load = vi.fn(async (): Promise<LiveHostData> => makeData());
      const { unmount } = render(
        <LiveInteractiveHost
          instanceId="inst-1"
          onUseStandardView={() => undefined}
          load={load}
          processState={{ phase: c.phase, instanceId: 'inst-1' }}
        />,
      );
      await waitFor(() => expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1));
      const bench = screen.getByTestId('world-editor');
      expect(bench).toHaveAttribute('data-launch-state', c.launch);
      expect(bench).toHaveAttribute('data-lock-state', c.busy ? 'busy' : 'editable');
      unmount();
    }
  });

  it('ignores canonical process state for a different instance', async () => {
    const load = vi.fn(async (): Promise<LiveHostData> => makeData());
    render(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        load={load}
        processState={{ phase: 'running', instanceId: 'inst-2' }}
      />,
    );
    await waitFor(() => expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1));
    const bench = screen.getByTestId('world-editor');
    expect(bench).toHaveAttribute('data-launch-state', 'idle');
  });

  it('blocks a review while an install is active (canonical availability)', async () => {
    const scene = baseScene();
    scene.findings = [
      {
        id: 'f1',
        severity: 'blocker',
        title: 'Loader does not fit',
        summary: 'Incompatible.',
        affectedIds: [],
        structuredKind: 'loader-compatibility',
        reviewIntent: { kind: 'review-loader' },
      },
    ];
    const load = vi.fn(async (): Promise<LiveHostData> => makeData(scene));
    const onOpenStandardOperation = vi.fn();
    render(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        onOpenStandardOperation={onOpenStandardOperation}
        load={load}
        capabilities={reviewCaps()}
        installActive
      />,
    );
    await waitFor(() => expect(screen.getByText('Loader does not fit')).toBeInTheDocument());
    const reviewButtons = screen.queryAllByRole('button', { name: 'Review' });
    for (const button of reviewButtons) fireEvent.click(button);
    expect(onOpenStandardOperation).not.toHaveBeenCalled();
  });

  it('routes review intents to the Standard bridge with context (never executes here)', async () => {
    const onOpenStandardOperation = vi.fn();
    const scene = baseScene();
    scene.findings = [
      {
        id: 'f1',
        severity: 'blocker',
        title: 'Loader does not fit',
        summary: 'Incompatible.',
        affectedIds: [],
        structuredKind: 'loader-compatibility',
        reviewIntent: { kind: 'review-loader' },
      },
    ];
    const load = vi.fn(async (): Promise<LiveHostData> => makeData(scene));
    render(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        onOpenStandardOperation={onOpenStandardOperation}
        load={load}
        capabilities={reviewCaps()}
      />,
    );
    await waitFor(() => expect(screen.getByText('Loader does not fit')).toBeInTheDocument());
    fireEvent.click(screen.getAllByRole('button', { name: 'Review' })[0]);
    expect(onOpenStandardOperation).toHaveBeenCalledWith({
      bridge: 'loader-review',
      context: { kind: 'loader-review', instanceId: 'inst-1' },
    });
  });

  it('Use Standard view invokes the escape immediately', async () => {
    const onUseStandardView = vi.fn();
    const load = vi.fn(async (): Promise<LiveHostData> => makeData());
    render(<LiveInteractiveHost instanceId="inst-1" onUseStandardView={onUseStandardView} load={load} />);
    await waitFor(() => expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1));
    fireEvent.click(screen.getByRole('button', { name: /Use Standard view/ }));
    expect(onUseStandardView).toHaveBeenCalledTimes(1);
  });

  it('canonical busy is reversible: idle -> launching -> idle clears busy (never sticky)', async () => {
    const load = vi.fn(async (): Promise<LiveHostData> => makeData());
    const { rerender } = render(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        load={load}
        processState={{ phase: 'idle', instanceId: 'inst-1' }}
      />,
    );
    await waitFor(() => expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1));
    let bench = screen.getByTestId('world-editor');
    expect(bench).toHaveAttribute('data-launch-state', 'idle');
    expect(bench).toHaveAttribute('data-lock-state', 'editable');

    rerender(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        load={load}
        processState={{ phase: 'launching', instanceId: 'inst-1' }}
      />,
    );
    bench = screen.getByTestId('world-editor');
    expect(bench).toHaveAttribute('data-launch-state', 'starting');
    expect(bench).toHaveAttribute('data-lock-state', 'busy');

    // Back to idle: busy must CLEAR (base read lock state is restored).
    rerender(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        load={load}
        processState={{ phase: 'idle', instanceId: 'inst-1' }}
      />,
    );
    bench = screen.getByTestId('world-editor');
    expect(bench).toHaveAttribute('data-launch-state', 'idle');
    expect(bench).toHaveAttribute('data-lock-state', 'editable');
  });

  it('install active -> inactive clears busy (only running installs are active)', async () => {
    const load = vi.fn(async (): Promise<LiveHostData> => makeData());
    const { rerender } = render(
      <LiveInteractiveHost instanceId="inst-1" onUseStandardView={() => undefined} load={load} installActive />,
    );
    await waitFor(() => expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1));
    let bench = screen.getByTestId('world-editor');
    expect(bench).toHaveAttribute('data-lock-state', 'busy');

    rerender(<LiveInteractiveHost instanceId="inst-1" onUseStandardView={() => undefined} load={load} installActive={false} />);
    bench = screen.getByTestId('world-editor');
    expect(bench).toHaveAttribute('data-lock-state', 'editable');
  });

  it('applies the LATEST canonical state when a read lands after a canonical change', async () => {
    let resolveLoad: (d: LiveHostData) => void = () => undefined;
    const load = vi.fn((): Promise<LiveHostData> => new Promise((resolve) => { resolveLoad = resolve; }));
    const { rerender } = render(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        load={load}
        processState={{ phase: 'idle', instanceId: 'inst-1' }}
      />,
    );
    await waitFor(() => expect(load).toHaveBeenCalledTimes(1));
    // Canonical state changes while the initial read is unresolved.
    rerender(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        load={load}
        processState={{ phase: 'running', instanceId: 'inst-1' }}
      />,
    );
    resolveLoad(makeData());
    await waitFor(() => expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1));
    // The accepted read is projected with the LATEST canonical state, not the
    // stale idle captured when the read started.
    const bench = screen.getByTestId('world-editor');
    expect(bench).toHaveAttribute('data-launch-state', 'running');
    expect(bench).toHaveAttribute('data-lock-state', 'busy');
  });

  it('two genuinely overlapping refreshes: newest wins, oldest discarded', async () => {
    let resolveR2: (d: LiveHostData) => void = () => undefined;
    let resolveR3: (d: LiveHostData) => void = () => undefined;
    let calls = 0;
    const load = vi.fn((): Promise<LiveHostData> => {
      calls += 1;
      if (calls === 1) return Promise.resolve(makeData());
      if (calls === 2) return new Promise((resolve) => { resolveR2 = resolve; });
      return new Promise((resolve) => { resolveR3 = resolve; });
    });
    render(<LiveInteractiveHost instanceId="inst-1" onUseStandardView={() => undefined} load={load} />);
    await waitFor(() => expect(calls).toBe(1));

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(calls).toBe(2));
    // Start a SECOND refresh while the first is STILL unresolved (both in flight).
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(calls).toBe(3));

    // Resolve the OLDER (second) refresh late with a spoofed name — discarded.
    resolveR2(makeData({ instance: { ...baseScene().instance!, name: 'OLD RESULT' } }));
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByText('OLD RESULT')).not.toBeInTheDocument();

    // Resolve the NEWEST (third) refresh — it is accepted.
    resolveR3(makeData());
    await waitFor(() => expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1));
    expect(screen.queryByText('OLD RESULT')).not.toBeInTheDocument();
  });

  it('records a review in flight (coalesces) and a MANUAL refresh does NOT clear it', async () => {
    const scene = baseScene();
    scene.findings = [
      {
        id: 'f1',
        severity: 'blocker',
        title: 'Loader does not fit',
        summary: 'Incompatible.',
        affectedIds: [],
        structuredKind: 'loader-compatibility',
        reviewIntent: { kind: 'review-loader' },
      },
      {
        id: 'f2',
        severity: 'warning',
        title: 'Health warning',
        summary: 'Needs a review.',
        affectedIds: [],
        structuredKind: 'runtime',
        reviewIntent: { kind: 'review-health' },
      },
    ];
    let resolveRefresh: (d: LiveHostData) => void = () => undefined;
    let calls = 0;
    const load = vi.fn((): Promise<LiveHostData> => {
      calls += 1;
      if (calls === 1) return Promise.resolve(makeData(scene));
      return new Promise((resolve) => { resolveRefresh = resolve; });
    });
    const onOpenStandardOperation = vi.fn();
    render(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        onOpenStandardOperation={onOpenStandardOperation}
        load={load}
        capabilities={reviewCaps()}
      />,
    );
    await waitFor(() => expect(screen.getByText('Loader does not fit')).toBeInTheDocument());

    // First review is dispatched and recorded as in-flight.
    fireEvent.click(screen.getAllByRole('button', { name: 'Review' })[0]);
    expect(onOpenStandardOperation).toHaveBeenCalledTimes(1);

    // A second review while the first is in flight is COALESCED (blocked).
    for (const button of screen.queryAllByRole('button', { name: 'Review' })) fireEvent.click(button);
    expect(onOpenStandardOperation).toHaveBeenCalledTimes(1);

    // A MANUAL refresh is NOT a review terminal event: the in-flight marker
    // survives the accepted read, so the second review stays coalesced
    // (SOL-2 §18.4 — never clear a proposal merely because the user refreshed).
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(calls).toBe(2));
    resolveRefresh(makeData(scene));
    await waitFor(() => expect(screen.getByText('Loader does not fit')).toBeInTheDocument());
    for (const button of screen.queryAllByRole('button', { name: 'Review' })) fireEvent.click(button);
    expect(onOpenStandardOperation).toHaveBeenCalledTimes(1);

    // Leaving High Interaction (host unmount) is the real terminal event — see
    // the next test: re-entry performs a fresh read with no stale in-flight
    // state.
  });

  it('re-entering High Interaction (remount) performs a fresh read with no stale in-flight state', async () => {
    const scene = baseScene();
    scene.findings = [
      {
        id: 'f1',
        severity: 'blocker',
        title: 'Loader does not fit',
        summary: 'Incompatible.',
        affectedIds: [],
        structuredKind: 'loader-compatibility',
        reviewIntent: { kind: 'review-loader' },
      },
    ];
    let calls = 0;
    const load = vi.fn(async (): Promise<LiveHostData> => {
      calls += 1;
      return makeData(scene);
    });
    const onOpenStandardOperation = vi.fn();
    const { unmount } = render(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        onOpenStandardOperation={onOpenStandardOperation}
        load={load}
        capabilities={reviewCaps()}
      />,
    );
    await waitFor(() => expect(screen.getByText('Loader does not fit')).toBeInTheDocument());
    fireEvent.click(screen.getAllByRole('button', { name: 'Review' })[0]);
    expect(onOpenStandardOperation).toHaveBeenCalledTimes(1);

    // The Standard surface leaves High Interaction (host unmounts), then the
    // user re-enters — a brand-new host instance reads fresh from the start.
    unmount();
    render(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        onOpenStandardOperation={onOpenStandardOperation}
        load={load}
        capabilities={reviewCaps()}
      />,
    );
    await waitFor(() => expect(screen.getByText('Loader does not fit')).toBeInTheDocument());
    expect(calls).toBeGreaterThanOrEqual(2); // fresh read on re-entry
    // No stale in-flight state: a review is dispatchable again.
    fireEvent.click(screen.getAllByRole('button', { name: 'Review' })[0]);
    expect(onOpenStandardOperation).toHaveBeenCalledTimes(2);
  });

  it('blocks review while the instance is locked by a player or recovery is pending/failed', async () => {
    const findings = [
      {
        id: 'f1',
        severity: 'blocker',
        title: 'Loader does not fit',
        summary: 'Incompatible.',
        affectedIds: [],
        structuredKind: 'loader-compatibility',
        reviewIntent: { kind: 'review-loader' },
      },
    ];
    const cases: Array<{ lockState: 'editable' | 'locked-by-player' | 'busy'; recovery: 'ready' | 'preparing' | 'failed'; expected: boolean }> = [
      { lockState: 'editable', recovery: 'ready', expected: true },
      { lockState: 'locked-by-player', recovery: 'ready', expected: false },
      { lockState: 'editable', recovery: 'preparing', expected: false },
      { lockState: 'editable', recovery: 'failed', expected: false },
    ];
    for (const c of cases) {
      const scene = baseScene();
      scene.findings = findings as typeof scene.findings;
      const load = vi.fn(async (): Promise<LiveHostData> =>
        makeData({
          ...scene,
          instance: { ...scene.instance!, lockState: c.lockState, recoveryReadiness: c.recovery },
        }),
      );
      const onOpenStandardOperation = vi.fn();
      const { unmount } = render(
        <LiveInteractiveHost
          instanceId="inst-1"
          onUseStandardView={() => undefined}
          onOpenStandardOperation={onOpenStandardOperation}
          load={load}
          capabilities={reviewCaps()}
        />,
      );
      await waitFor(() => expect(screen.getByText('Loader does not fit')).toBeInTheDocument());
      for (const button of screen.queryAllByRole('button', { name: 'Review' })) fireEvent.click(button);
      expect(onOpenStandardOperation).toHaveBeenCalledTimes(c.expected ? 1 : 0);
      unmount();
    }
  });

  it('enriches the install-flow route with backend-derived filename/kind before dispatch', async () => {
    const scene = baseScene();
    scene.content = [contentNode()];
    const load = vi.fn(async (): Promise<LiveHostData> => makeData(scene));
    const onOpenStandardOperation = vi.fn();
    render(
      <LiveInteractiveHost
        instanceId="inst-1"
        onUseStandardView={() => undefined}
        onOpenStandardOperation={onOpenStandardOperation}
        load={load}
        capabilities={reviewCaps()}
      />,
    );
    await waitFor(() => expect(screen.getAllByText('sodium.jar').length).toBeGreaterThanOrEqual(1));
    fireEvent.click(screen.getByRole('button', { name: /^Stage install:/ }));
    expect(onOpenStandardOperation).toHaveBeenCalledWith({
      bridge: 'install-flow',
      context: {
        kind: 'install-flow',
        instanceId: 'inst-1',
        action: 'install',
        contentId: 'live:content:mod:sodium.jar',
        filename: 'sodium.jar',
        contentKind: 'mod',
      },
    });
  });

  it('withholds the old scene (non-routable) while an instance switch is unresolved (SOL-2 §19.4)', async () => {
    let resolveB: (d: LiveHostData) => void = () => undefined;
    const load = vi.fn((id: string): Promise<LiveHostData> => {
      if (id === 'inst-A') return Promise.resolve(makeData());
      return new Promise((resolve) => { resolveB = resolve; }); // B unresolved
    });
    const onOpenStandardOperation = vi.fn();
    const { rerender } = render(
      <LiveInteractiveHost
        instanceId="inst-A"
        onUseStandardView={() => undefined}
        onOpenStandardOperation={onOpenStandardOperation}
        load={load}
        capabilities={reviewCaps()}
      />,
    );
    await waitFor(() => expect(screen.getAllByText('My World').length).toBeGreaterThanOrEqual(1));

    // Switch to B while B is unresolved: the A scene must be withheld by the
    // RENDER guard (not a passive effect) — loading, non-routable.
    rerender(
      <LiveInteractiveHost
        instanceId="inst-B"
        onUseStandardView={() => undefined}
        onOpenStandardOperation={onOpenStandardOperation}
        load={load}
        capabilities={reviewCaps()}
      />,
    );
    await waitFor(() => expect(screen.queryByText('My World')).not.toBeInTheDocument());
    expect(screen.getByText('Loading live data…')).toBeInTheDocument();

    // Resolve B; only then does the B scene render.
    resolveB(makeData({ instance: { ...baseScene().instance!, id: 'inst-B', name: 'World B' } }));
    await waitFor(() => expect(screen.getAllByText('World B').length).toBeGreaterThanOrEqual(1));
    expect(onOpenStandardOperation).not.toHaveBeenCalled();
  });

  it('never routes old instance content to the newly selected instance during a switch (SOL-2 §19.4)', async () => {
    const sceneA = baseScene();
    sceneA.content = [contentNode()]; // A has a staged-install node (shared filename risk)
    let resolveB: (d: LiveHostData) => void = () => undefined;
    const load = vi.fn((id: string): Promise<LiveHostData> => {
      if (id === 'inst-A') return Promise.resolve(makeData(sceneA));
      return new Promise((resolve) => { resolveB = resolve; }); // B unresolved
    });
    const onOpenStandardOperation = vi.fn();
    const { rerender } = render(
      <LiveInteractiveHost
        instanceId="inst-A"
        onUseStandardView={() => undefined}
        onOpenStandardOperation={onOpenStandardOperation}
        load={load}
        capabilities={reviewCaps()}
      />,
    );
    await waitFor(() => expect(screen.getByRole('button', { name: /^Stage install:/ })).toBeInTheDocument());

    // Switch to B while unresolved: A's content is withheld entirely — there is
    // no Stage button to click and no scene to route, so no B bridge/InstallIntent
    // can be emitted from A data.
    rerender(
      <LiveInteractiveHost
        instanceId="inst-B"
        onUseStandardView={() => undefined}
        onOpenStandardOperation={onOpenStandardOperation}
        load={load}
        capabilities={reviewCaps()}
      />,
    );
    await waitFor(() => expect(screen.queryByRole('button', { name: /^Stage install:/ })).not.toBeInTheDocument());
    expect(screen.queryByText('sodium.jar')).not.toBeInTheDocument();
    expect(screen.getByText('Loading live data…')).toBeInTheDocument();
    expect(onOpenStandardOperation).not.toHaveBeenCalled();

    // Resolve B with a node that SHARES the filename; a fresh install gesture
    // routes to B's instance id (never A's).
    const sceneB = baseScene();
    sceneB.instance = { ...baseScene().instance!, id: 'inst-B', name: 'World B' };
    sceneB.content = [contentNode({ id: 'live:content:mod:sodium.jar', name: 'sodium.jar' })];
    resolveB(makeData(sceneB));
    await waitFor(() => expect(screen.getByRole('button', { name: /^Stage install:/ })).toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: /^Stage install:/ }));
    expect(onOpenStandardOperation).toHaveBeenCalledWith({
      bridge: 'install-flow',
      context: {
        kind: 'install-flow',
        instanceId: 'inst-B',
        action: 'install',
        contentId: 'live:content:mod:sodium.jar',
        filename: 'sodium.jar',
        contentKind: 'mod',
      },
    });
  });
});
