import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { HealthLens } from './HealthLens';
import { LoaderRail } from './LoaderRail';
import { RuntimeWorkbench } from './RuntimeWorkbench';
import { CrashEvidenceBoard } from './CrashEvidenceBoard';
import { NetworkReadinessMap } from './NetworkReadinessMap';
import { InstanceBench } from './InstanceBench';
import { NO_CAPABILITIES } from '../domain/models';
import type {
  VisualCrashEvidence,
  VisualHealthFinding,
  VisualLoaderCandidate,
  VisualNetworkReadiness,
  VisualRuntimeState,
} from '../domain/models';

const source = { kind: 'simulation' as const, scenarioId: 'test', scenarioVersion: 1 };

const findings: VisualHealthFinding[] = [
  {
    id: 'lab:heal:finding:loader',
    severity: 'blocker',
    title: 'The loader does not fit',
    summary: 'Incompatible with two mods.',
    affectedIds: [],
    structuredKind: 'loader-compatibility',
    compatibility: 'incompatible',
    reviewIntent: { kind: 'review-loader' },
  },
  {
    id: 'lab:heal:finding:java',
    severity: 'warning',
    title: 'Java version needs attention',
    summary: 'Too old for this Minecraft version.',
    affectedIds: [],
    structuredKind: 'runtime',
  },
  {
    id: 'lab:heal:finding:memory',
    severity: 'recommendation',
    title: 'Memory: automatic recommended',
    summary: 'Automatic is fine here.',
    affectedIds: [],
    structuredKind: 'runtime',
  },
];

const candidates: VisualLoaderCandidate[] = [
  {
    id: 'lab:heal:loader:current',
    family: 'Forge',
    version: '47.1',
    channel: 'stable',
    role: 'current',
    compatibility: 'incompatible',
    requirementSummary: { satisfied: 1, indeterminate: 0, failed: 2 },
    affectedContent: { visibleNames: ['Tweakeroo'], total: 2 },
    explanation: 'Does not fit.',
  },
  {
    id: 'lab:heal:loader:fabric',
    family: 'Fabric',
    version: '0.15.11',
    channel: 'stable',
    role: 'recommended',
    compatibility: 'compatible',
    requirementSummary: { satisfied: 3, indeterminate: 0, failed: 0 },
    affectedContent: { visibleNames: [], total: 0 },
    explanation: 'Proven compatible.',
  },
];

const runtime: VisualRuntimeState = {
  runtime: { currentLabel: 'Java 8', requiredJavaMajor: 17, compatibility: 'incompatible', managedByAgora: false },
  memory: {
    mode: { current: 'automatic' },
    currentMiB: 2048,
    recommendedMiB: 4096,
    safeHeadroomLabel: '12 GB free of 16 GB',
    explanation: 'Agora chooses automatically.',
  },
  garbageCollector: { current: { mode: 'automatic' } },
  availability: 'available',
};

const evidence: VisualCrashEvidence = {
  incidentLabel: 'Crash on launch — Aug 9',
  evidenceSources: [
    { kind: 'crash-report', state: 'known', summary: 'Names a mod class.' },
    { kind: 'log', state: 'known', summary: 'Out-of-memory line.' },
  ],
  hypotheses: [
    {
      id: 'lab:fix:hyp:mod',
      title: 'A mod fails during startup',
      strength: 'high',
      supportingClues: ['Crash report names a mod class.'],
      contradictoryClues: [],
      state: 'candidate',
    },
  ],
  experiment: { phase: 'read-only', recoveryReady: true },
  privacyNote: 'Evidence stays on this device.',
};

const readiness: VisualNetworkReadiness = {
  instanceName: 'My Redstone World',
  launchMode: 'delegated',
  policy: 'normal',
  overall: 'needs-attention',
  checkedAt: 'just now',
  checks: [
    { id: 'lab:offline:check:game-files', category: 'game-files', label: 'Game files', state: 'ready', summary: 'Downloaded.' },
    { id: 'lab:offline:check:content', category: 'content', label: 'Content', state: 'missing', summary: 'One file missing.' },
    { id: 'lab:offline:check:sign-in', category: 'sign-in-and-launch', label: 'Sign-in', state: 'ready', summary: 'Delegated.' },
  ],
};

describe('HealthLens', () => {
  it('shows the severity hierarchy and validated summary', () => {
    render(
      <HealthLens findings={findings} source={source} validated selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={NO_CAPABILITIES} />,
    );
    expect(screen.getByText('1 blocker')).toBeInTheDocument();
    expect(screen.getByText('1 warning')).toBeInTheDocument();
    expect(screen.getByText('1 recommendation')).toBeInTheDocument();
    expect(screen.getByText('Blockers')).toBeInTheDocument();
    expect(screen.getByText('Warnings')).toBeInTheDocument();
    expect(screen.getByText('Recommendations')).toBeInTheDocument();
  });

  it('emits the finding review intent from the Review button', () => {
    const onIntent = vi.fn();
    const caps = { ...NO_CAPABILITIES, canReviewLoader: true };
    render(
      <HealthLens findings={findings} source={source} validated selection={null} onSelect={() => undefined} onIntent={onIntent} capabilities={caps} />,
    );
    fireEvent.click(screen.getAllByRole('button', { name: 'Review' })[0]);
    expect(onIntent).toHaveBeenCalledWith({ kind: 'review-loader' });
  });

  it('prompts to run validation before a scan', () => {
    render(
      <HealthLens findings={[]} source={source} validated={false} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={NO_CAPABILITIES} />,
    );
    expect(screen.getByText(/Run the validation check/)).toBeInTheDocument();
  });
});

describe('LoaderRail', () => {
  it('shows role, compatibility, and requirement counts', () => {
    render(
      <LoaderRail candidates={candidates} source={source} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={NO_CAPABILITIES} />,
    );
    expect(screen.getByText('Forge 47.1')).toBeInTheDocument();
    expect(screen.getByText('Fabric 0.15.11')).toBeInTheDocument();
    expect(screen.getByText('Incompatible')).toBeInTheDocument();
    expect(screen.getByText('Compatible')).toBeInTheDocument();
    expect(screen.getByText(/1 satisfied · 0 uncertain · 2 failed/)).toBeInTheDocument();
  });

  it('emits review-loader intent when capability is enabled', () => {
    const onIntent = vi.fn();
    const caps = { ...NO_CAPABILITIES, canReviewLoader: true };
    render(
      <LoaderRail candidates={candidates} source={source} selection={null} onSelect={() => undefined} onIntent={onIntent} capabilities={caps} />,
    );
    fireEvent.click(screen.getAllByRole('button', { name: 'Review this loader' })[1]);
    expect(onIntent).toHaveBeenCalledWith({ kind: 'review-loader', candidateId: 'lab:heal:loader:fabric' });
  });
});

describe('RuntimeWorkbench', () => {
  it('shows runtime compatibility and memory guidance', () => {
    render(
      <RuntimeWorkbench runtime={runtime} source={source} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={NO_CAPABILITIES} />,
    );
    expect(screen.getByText('Java 8')).toBeInTheDocument();
    expect(screen.getByText('needs Java 17+')).toBeInTheDocument();
    expect(screen.getByText('Incompatible')).toBeInTheDocument();
    expect(screen.getByText(/recommended 4 GB/)).toBeInTheDocument();
    expect(screen.getByText(/12 GB free of 16 GB/)).toBeInTheDocument();
  });

  it('stages a manual memory proposal through the intent', () => {
    const onIntent = vi.fn();
    const caps = { ...NO_CAPABILITIES, canProposeMemory: true };
    render(
      <RuntimeWorkbench runtime={runtime} source={source} selection={null} onSelect={() => undefined} onIntent={onIntent} capabilities={caps} />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Stage manual choice' }));
    expect(onIntent).toHaveBeenCalledWith({ kind: 'propose-memory', mode: 'manual', memoryMiB: 4096 });
  });

  it('labels an indeterminate Java runtime explicitly instead of a bare ambiguous Unknown chip (TERRA-5 P2)', () => {
    const unknownRuntime: VisualRuntimeState = {
      ...runtime,
      runtime: { currentLabel: 'System Java', requiredJavaMajor: 17, compatibility: 'unknown', managedByAgora: false },
    };
    render(
      <RuntimeWorkbench runtime={unknownRuntime} source={source} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={NO_CAPABILITIES} />,
    );
    // Explicit, tied to the Runtime row — not a floating "Unknown" chip.
    expect(screen.getByText('Java runtime: not verified')).toBeInTheDocument();
    expect(screen.queryByText('Unknown')).not.toBeInTheDocument();
    // The confident memory recommendation still reads clearly beside it.
    expect(screen.getByText(/recommended 4 GB/)).toBeInTheDocument();
  });
});

describe('CrashEvidenceBoard', () => {
  it('shows clues, hypotheses with strength, and the experiment region', () => {
    render(
      <CrashEvidenceBoard evidence={evidence} source={source} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={NO_CAPABILITIES} />,
    );
    expect(screen.getByText('Crash on launch — Aug 9')).toBeInTheDocument();
    expect(screen.getByText('A mod fails during startup')).toBeInTheDocument();
    expect(screen.getByText(/Strength: High/)).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Experiment' })).toBeInTheDocument();
    expect(screen.getByText(/Evidence stays on this device/)).toBeInTheDocument();
  });
});

describe('NetworkReadinessMap', () => {
  it('shows per-category states and overall readiness', () => {
    render(
      <NetworkReadinessMap readiness={readiness} source={source} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={NO_CAPABILITIES} />,
    );
    expect(screen.getByText(/Offline readiness — My Redstone World/)).toBeInTheDocument();
    expect(screen.getAllByText('Ready').length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText('Missing')).toBeInTheDocument();
    expect(screen.getByText(/Overall: Needs attention/)).toBeInTheDocument();
  });

  it('emits review-offline-readiness from the re-check control', () => {
    const onIntent = vi.fn();
    const caps = { ...NO_CAPABILITIES, canReviewOfflineReadiness: true };
    render(
      <NetworkReadinessMap readiness={readiness} source={source} selection={null} onSelect={() => undefined} onIntent={onIntent} capabilities={caps} />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Re-check readiness' }));
    expect(onIntent).toHaveBeenCalledWith({ kind: 'review-offline-readiness' });
  });
});

/** TERRA-6 fixes verified at the shared-visual layer. */
describe('TERRA-6 shared-visual fixes', () => {
  it('HealthLens does not list findings before a scan has run (T6-8)', () => {
    const findings = [
      { id: 'f1', severity: 'blocker' as const, title: 'A blocker', summary: 'Details', affectedIds: [] },
    ];
    const { rerender } = render(
      <HealthLens
        findings={findings}
        validated={false}
        source={{ kind: 'simulation', scenarioId: 'x', scenarioVersion: 1 }}
        selection={null}
        onSelect={() => undefined}
        onIntent={() => undefined}
        capabilities={NO_CAPABILITIES}
      />,
    );
    expect(screen.queryByText('A blocker')).not.toBeInTheDocument();
    expect(screen.getByText(/Run the validation check/)).toBeInTheDocument();

    rerender(
      <HealthLens
        findings={findings}
        validated
        source={{ kind: 'simulation', scenarioId: 'x', scenarioVersion: 1 }}
        selection={null}
        onSelect={() => undefined}
        onIntent={() => undefined}
        capabilities={NO_CAPABILITIES}
      />,
    );
    expect(screen.getByText('A blocker')).toBeInTheDocument();
  });

  it('InstanceBench marks the current loader in every compatibility state (T6-3)', () => {
    const bench = (compatibility: 'compatible' | 'incompatible' | 'unknown' | 'indeterminate') => ({
      id: 'i1',
      name: 'Test',
      gameVersion: '1.21.1',
      loader: { current: { family: 'fabric', compatibility } },
      lockState: 'editable' as const,
      recoveryReadiness: 'unknown' as const,
      launchState: 'idle' as const,
      contentSummary: { enabled: 1, disabled: 0, needsAttention: 0 },
    });
    const props = {
      source: {
        kind: 'live' as const,
        viewRevision: 'r1',
        observedAt: '2026-08-11T00:00:00Z',
        freshness: 'fresh' as const,
      },
      selection: null,
      onSelect: () => undefined,
      onIntent: () => undefined,
      capabilities: NO_CAPABILITIES,
    };
    // The live adapter always reports `unknown`, so this is the normal case for
    // every real instance — it must never render as silence.
    const { rerender } = render(<InstanceBench instance={bench('unknown')} {...props} />);
    expect(screen.getByText('Not verified')).toBeInTheDocument();

    rerender(<InstanceBench instance={bench('incompatible')} {...props} />);
    expect(screen.getByText('Does not fit this setup')).toBeInTheDocument();

    rerender(<InstanceBench instance={bench('indeterminate')} {...props} />);
    expect(screen.getByText(/Needs review/)).toBeInTheDocument();

    rerender(<InstanceBench instance={bench('compatible')} {...props} />);
    expect(screen.getByText('Fits this setup')).toBeInTheDocument();
  });
});
