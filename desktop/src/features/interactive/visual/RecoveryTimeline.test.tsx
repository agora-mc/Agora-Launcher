import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { RecoveryTimeline } from './RecoveryTimeline';
import { NO_CAPABILITIES } from '../domain/models';
import type { VisualSnapshot } from '../domain/models';

const snapshots: VisualSnapshot[] = [
  {
    id: 'lab:undo:snap:auto',
    label: 'Automatic return point',
    createdAt: 'Today, 09:00',
    role: 'automatic',
    sizeLabel: 'Fast',
    protects: ['mods', 'config', 'other-instance-files'],
    worldProtection: 'not-included',
    availability: 'available',
  },
  {
    id: 'lab:undo:snap:manual',
    label: 'Weekend manual snapshot',
    createdAt: '3 days ago',
    role: 'manual',
    sizeLabel: 'Large',
    changeSummary: { added: 3, changed: 1, removed: 2 },
    protects: ['mods', 'config', 'worlds', 'other-instance-files'],
    worldProtection: 'included',
    availability: 'available',
  },
];

const source = { kind: 'simulation' as const, scenarioId: 'undo', scenarioVersion: 1 };

describe('RecoveryTimeline', () => {
  it('renders return points with role, scope, and world boundary', () => {
    render(
      <RecoveryTimeline
        snapshots={snapshots}
        currentStateLabel="Current state — a change made things worse"
        source={source}
        selection={null}
        onSelect={() => undefined}
        onIntent={() => undefined}
        capabilities={NO_CAPABILITIES}
      />,
    );
    expect(screen.getByText('Automatic return point')).toBeInTheDocument();
    expect(screen.getByText(/worlds NOT included/)).toBeInTheDocument();
    expect(screen.getByText('Weekend manual snapshot')).toBeInTheDocument();
    expect(screen.getByText(/worlds included/)).toBeInTheDocument();
    expect(screen.getByText(/Current state — a change made things worse/)).toBeInTheDocument();
  });

  it('emits a preview intent when Compare is chosen', () => {
    const onIntent = vi.fn();
    render(
      <RecoveryTimeline
        snapshots={snapshots}
        currentStateLabel="Current"
        source={source}
        selection="lab:undo:snap:manual"
        onSelect={() => undefined}
        onIntent={onIntent}
        capabilities={NO_CAPABILITIES}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Compare' }));
    expect(onIntent).toHaveBeenCalledWith({ kind: 'preview-snapshot', snapshotId: 'lab:undo:snap:manual' });
  });

  it('only offers restore when the capability is enabled', () => {
    render(
      <RecoveryTimeline
        snapshots={snapshots}
        currentStateLabel="Current"
        source={source}
        selection="lab:undo:snap:manual"
        onSelect={() => undefined}
        onIntent={() => undefined}
        capabilities={NO_CAPABILITIES}
      />,
    );
    expect(screen.queryByRole('button', { name: /Restore/ })).not.toBeInTheDocument();

    render(
      <RecoveryTimeline
        snapshots={snapshots}
        currentStateLabel="Current"
        source={source}
        selection="lab:undo:snap:manual"
        onSelect={() => undefined}
        onIntent={() => undefined}
        capabilities={{ ...NO_CAPABILITIES, canRequestSnapshotRestore: true }}
      />,
    );
    expect(screen.getByRole('button', { name: /Restore…/ })).toBeInTheDocument();
  });
});
