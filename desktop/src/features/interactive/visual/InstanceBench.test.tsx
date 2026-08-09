import { describe, expect, it } from 'vitest';
import { useState } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { InstanceBench } from './InstanceBench';
import { NO_CAPABILITIES } from '../domain/models';
import type { VisualId, VisualInstance } from '../domain/models';

const instance: VisualInstance = {
  id: 'lab:build:instance:mine',
  name: 'My Redstone World',
  gameVersion: '1.20.1',
  loader: {
    current: { family: 'Fabric', version: '0.15', compatibility: 'compatible' },
    proposed: { family: 'Fabric', version: '0.16', compatibility: 'indeterminate' },
  },
  lockState: 'editable',
  recoveryReadiness: 'unknown',
  launchState: 'idle',
  contentSummary: { enabled: 1, disabled: 0, needsAttention: 0 },
};

const source = { kind: 'simulation' as const, scenarioId: 'build', scenarioVersion: 1 };

describe('InstanceBench', () => {
  it('renders current values and a proposed loader with an explicit label', () => {
    render(
      <InstanceBench
        instance={instance}
        source={source}
        selection={null}
        onSelect={() => undefined}
        onIntent={() => undefined}
        capabilities={NO_CAPABILITIES}
      />,
    );
    expect(screen.getByText('My Redstone World')).toBeInTheDocument();
    expect(screen.getByText('1.20.1')).toBeInTheDocument();
    expect(screen.getAllByText('Proposed').length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText('Needs review')).toBeInTheDocument(); // indeterminate loader proposed
    expect(screen.getByText('Simulation')).toBeInTheDocument();
  });

  it('toggles selection on click (controlled component)', () => {
    function Harness() {
      const [selection, setSelection] = useState<VisualId | null>(null);
      return (
        <InstanceBench
          instance={instance}
          source={source}
          selection={selection}
          onSelect={setSelection}
          onIntent={() => undefined}
          capabilities={NO_CAPABILITIES}
        />
      );
    }
    render(<Harness />);
    fireEvent.click(screen.getByText('My Redstone World'));
    expect(screen.getByRole('button', { name: /My Redstone World/ })).toHaveAttribute('aria-pressed', 'true');
    fireEvent.click(screen.getByText('My Redstone World'));
    expect(screen.getByRole('button', { name: /My Redstone World/ })).toHaveAttribute('aria-pressed', 'false');
  });

  it('shows content summary counts', () => {
    render(
      <InstanceBench
        instance={instance}
        source={source}
        selection={null}
        onSelect={() => undefined}
        onIntent={() => undefined}
        capabilities={NO_CAPABILITIES}
      />,
    );
    expect(screen.getByText('1')).toBeInTheDocument(); // enabled
    expect(screen.getByText('Enabled')).toBeInTheDocument();
  });

  it('renders a role label and highlight for the working instance', () => {
    const plainInstance: VisualInstance = {
      ...instance,
      loader: { current: { family: 'Fabric', compatibility: 'compatible' } },
    };
    render(
      <InstanceBench
        instance={plainInstance}
        source={source}
        selection={null}
        onSelect={() => undefined}
        onIntent={() => undefined}
        capabilities={NO_CAPABILITIES}
        roleLabel="Your new instance"
        highlight
      />,
    );
    expect(screen.getByRole('region', { name: 'Your new instance' })).toBeInTheDocument();
    expect(screen.getByText('Instance bench')).toBeInTheDocument();
    expect(screen.getByText('Current')).toBeInTheDocument(); // phase mark retained for the working instance
  });

  it('renders a status label for an unchanged sibling without a Current phase mark', () => {
    const plainInstance: VisualInstance = {
      ...instance,
      loader: { current: { family: 'Fabric', compatibility: 'compatible' } },
    };
    render(
      <InstanceBench
        instance={plainInstance}
        source={source}
        selection={null}
        onSelect={() => undefined}
        onIntent={() => undefined}
        capabilities={NO_CAPABILITIES}
        roleLabel="Existing example"
        statusLabel="Separate · unchanged"
      />,
    );
    expect(screen.getByRole('region', { name: 'Existing example' })).toBeInTheDocument();
    expect(screen.getByText('Separate · unchanged')).toBeInTheDocument();
    expect(screen.queryByText('Current')).not.toBeInTheDocument();
    expect(screen.queryByText('Proposed')).not.toBeInTheDocument();
  });
});
