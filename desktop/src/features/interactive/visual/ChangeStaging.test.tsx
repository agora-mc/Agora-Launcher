import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { ChangeStaging } from './ChangeStaging';
import { NO_CAPABILITIES } from '../domain/models';
import type { VisualProposal, VisualScene } from '../domain/models';

function scene(proposals: VisualProposal[]): VisualScene {
  return {
    source: { kind: 'simulation', scenarioId: 'mod', scenarioVersion: 1 },
    content: [],
    relationships: [],
    findings: [],
    proposals,
  };
}

const source = { kind: 'simulation' as const, scenarioId: 'mod', scenarioVersion: 1 };

describe('ChangeStaging', () => {
  it('shows an empty state when nothing is staged', () => {
    render(
      <ChangeStaging scene={scene([])} source={source} onIntent={() => undefined} capabilities={NO_CAPABILITIES} />,
    );
    expect(screen.getByText(/No changes staged/)).toBeInTheDocument();
  });

  it('renders staged proposals with phase marks and destructive warnings', () => {
    const proposals: VisualProposal[] = [
      {
        id: 'lab:mod:proposal',
        intent: { kind: 'propose-remove', contentId: 'lab:mod:terrain-overhaul' },
        phase: 'proposed',
        title: 'Remove Terrain Overhaul',
        summary: 'Replaces it to resolve a conflict.',
        destructive: true,
      },
    ];
    render(
      <ChangeStaging scene={scene(proposals)} source={source} onIntent={() => undefined} capabilities={NO_CAPABILITIES} />,
    );
    expect(screen.getByText('Remove Terrain Overhaul')).toBeInTheDocument();
    expect(screen.getByText('Proposed')).toBeInTheDocument();
    expect(screen.getByText(/Destructive/)).toBeInTheDocument();
    expect(screen.getByText(/1 destructive change/)).toBeInTheDocument();
  });

  it('emits a review-staged-changes intent from the review control', () => {
    const onIntent = vi.fn();
    const proposals: VisualProposal[] = [
      {
        id: 'lab:mod:proposal',
        intent: { kind: 'propose-install', contentId: 'lab:mod:better-caves' },
        phase: 'proposed',
        title: 'Install BetterCaves',
        summary: 'Adds the mod.',
        destructive: false,
      },
    ];
    render(
      <ChangeStaging
        scene={scene(proposals)}
        source={source}
        onIntent={onIntent}
        capabilities={NO_CAPABILITIES}
        reviewLabel="Apply simulated plan"
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Apply simulated plan' }));
    expect(onIntent).toHaveBeenCalledWith({ kind: 'review-staged-changes' });
  });

  it('renders an applied outcome instead of the empty message', () => {
    render(
      <ChangeStaging
        scene={scene([])}
        source={source}
        onIntent={() => undefined}
        capabilities={NO_CAPABILITIES}
        outcome={{
          title: 'Plan applied',
          summary: 'BetterCaves installed. Terrain Overhaul removed.',
          recoveryPoint: 'A simulated return point was created before applying.',
        }}
      />,
    );
    expect(screen.getByRole('region', { name: 'Applied outcome' })).toBeInTheDocument();
    expect(screen.getByText('Plan applied')).toBeInTheDocument();
    expect(screen.getByText(/BetterCaves installed/)).toBeInTheDocument();
    expect(screen.getByText(/return point was created/)).toBeInTheDocument();
    expect(screen.queryByText(/No changes staged/)).not.toBeInTheDocument();
  });

  it('hides the review control when review is not available', () => {
    const proposals: VisualProposal[] = [
      {
        id: 'lab:mod:proposal',
        intent: { kind: 'propose-install', contentId: 'lab:mod:better-caves' },
        phase: 'proposed',
        title: 'Install BetterCaves',
        summary: 'Adds the mod.',
        destructive: false,
      },
    ];
    render(
      <ChangeStaging
        scene={scene(proposals)}
        source={source}
        onIntent={() => undefined}
        capabilities={NO_CAPABILITIES}
        reviewLabel="Apply simulated plan"
        reviewAvailable={false}
      />,
    );
    expect(screen.queryByRole('button', { name: 'Apply simulated plan' })).not.toBeInTheDocument();
  });
});
