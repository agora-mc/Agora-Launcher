import { beforeEach, describe, expect, it } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { LabShell } from './LabShell';

beforeEach(() => {
  window.localStorage.clear();
});

const noop = () => undefined;

function renderShell() {
  return render(
    <LabShell
      onOpenGuide={() => undefined}
      onNavigateStandard={noop}
      guideTopicLabels={{ instances: 'Instances: Your Isolated Worlds' }}
    />,
  );
}

describe('LabShell', () => {
  it('shows the adventure selection screen with a persistent Simulation identity', () => {
    renderShell();
    expect(screen.getByText('Agora Lab')).toBeInTheDocument();
    expect(screen.getByText('Build It')).toBeInTheDocument();
    expect(screen.getByText('Mod It')).toBeInTheDocument();
    expect(screen.getByText('Undo It')).toBeInTheDocument();
    expect(screen.getByText(/Nothing here changes your real instances/)).toBeInTheDocument();
  });

  it('plays Build It end-to-end to completion with feedback', () => {
    renderShell();
    const buildCard = screen.getByTestId('adventure-build');
    fireEvent.click(within(buildCard).getByRole('button', { name: 'Start' }));

    expect(screen.getAllByText('Simulation').length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText(/Step 1 of 3/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Place Minecraft 1.20.1' }));
    expect(screen.getByText('Minecraft 1.20.1 is placed on the bench.')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Choose Forge' }));
    expect(screen.getAllByText(/Forge does not fit 1.20.1/).length).toBeGreaterThanOrEqual(1);

    fireEvent.click(screen.getByRole('button', { name: 'Choose Fabric' }));
    fireEvent.click(screen.getByRole('button', { name: 'Name it "My Redstone World"' }));
    fireEvent.click(screen.getByRole('button', { name: 'Place Notebot Mod' }));

    expect(screen.getByText('Adventure complete')).toBeInTheDocument();
    expect(screen.getAllByText(/You completed the practice/).length).toBeGreaterThanOrEqual(1);
    expect(screen.getByRole('button', { name: /Field Guide: Instances/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'My Instances' })).toBeInTheDocument();
  });

  it('Reset restores the first checkpoint', () => {
    renderShell();
    fireEvent.click(within(screen.getByTestId('adventure-build')).getByRole('button', { name: 'Start' }));
    fireEvent.click(screen.getByRole('button', { name: 'Place Minecraft 1.20.1' }));
    fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
    expect(screen.getByText(/Step 1 of 3/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Place Minecraft 1.20.1' })).toBeInTheDocument();
  });

  it('Undo It requires a serious confirmation before restore', () => {
    renderShell();
    fireEvent.click(within(screen.getByTestId('adventure-undo')).getByRole('button', { name: 'Start' }));

    fireEvent.click(screen.getByRole('button', { name: 'Compare Last known good' }));
    fireEvent.click(screen.getByRole('button', { name: 'Restore Weekend manual snapshot (worlds included)' }));

    // running process blocks restore; feedback is visible inline AND in the status region
    fireEvent.click(screen.getByRole('button', { name: 'Try restore now' }));
    expect(screen.getAllByText(/Blocked: the instance is running/).length).toBeGreaterThanOrEqual(1);
    // the attempted action carries a blocked/error styling marker
    expect(screen.getByRole('button', { name: 'Try restore now' }).className).toContain('destructive');

    fireEvent.click(screen.getByRole('button', { name: 'Stop the simulated instance' }));

    // danger decision opens a confirmation dialog, not an immediate restore
    fireEvent.click(screen.getByRole('button', { name: 'Restore: Weekend manual snapshot' }));
    expect(screen.getByRole('alertdialog', { name: 'Confirm restore' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
    expect(screen.queryByText('Adventure complete')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Restore: Weekend manual snapshot' }));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));
    expect(screen.getByText('Adventure complete')).toBeInTheDocument();
    expect(screen.getByText('Undo point created')).toBeInTheDocument();
    // post-restore state is visibly different: restored current label, undo point on the timeline, outcome
    expect(screen.getByRole('region', { name: 'Restore outcome' })).toBeInTheDocument();
    expect(screen.getByText(/Current state — restored to/)).toBeInTheDocument();
    expect(screen.getAllByText('Undo restore point').length).toBeGreaterThanOrEqual(1);
  });

  it('Undo It: blocked attempt shows visible inline feedback next to the action', () => {
    renderShell();
    fireEvent.click(within(screen.getByTestId('adventure-undo')).getByRole('button', { name: 'Start' }));
    fireEvent.click(screen.getByRole('button', { name: 'Compare Last known good' }));
    fireEvent.click(screen.getByRole('button', { name: 'Restore Weekend manual snapshot (worlds included)' }));
    fireEvent.click(screen.getByRole('button', { name: 'Try restore now' }));

    expect(screen.getAllByText(/Blocked: the instance is running/).length).toBeGreaterThanOrEqual(1);
    expect(screen.getByRole('button', { name: 'Try restore now' }).className).toContain('destructive');
  });

  it('Mod It: replacement confirmation is action-specific and post-apply state is fully current', () => {
    renderShell();
    fireEvent.click(within(screen.getByTestId('adventure-mod')).getByRole('button', { name: 'Start' }));

    fireEvent.click(screen.getByRole('button', { name: 'Stage BetterCaves' }));
    fireEvent.click(screen.getByRole('button', { name: 'Snap required: Core Lib' }));
    fireEvent.click(screen.getByRole('button', { name: 'Include optional: Nice Textures' }));

    // Replacement danger decision opens a REPLACEMENT dialog, never a restore dialog
    fireEvent.click(screen.getByRole('button', { name: 'Replace Terrain Overhaul' }));
    expect(screen.getByRole('alertdialog', { name: 'Confirm replacement' })).toBeInTheDocument();
    expect(screen.queryByRole('alertdialog', { name: 'Confirm restore' })).not.toBeInTheDocument();
    expect(screen.getByText(/removes Terrain Overhaul/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }));

    // Apply appears in both the staging dock and the action list; click the first
    fireEvent.click(screen.getAllByRole('button', { name: 'Apply simulated plan' })[0]);

    // Post-apply: applied outcome shown, graph is fully current (no proposed markers)
    expect(screen.getByText('Adventure complete')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Applied outcome' })).toBeInTheDocument();
    expect(screen.getByText('Plan applied')).toBeInTheDocument();
    expect(screen.queryAllByText('Proposed').length).toBe(0);
    expect(screen.queryByText('Proposed: remove')).not.toBeInTheDocument();
    expect(screen.getAllByText('BetterCaves').length).toBeGreaterThanOrEqual(1);
  });

  it('Exit returns to the adventure selection', () => {
    renderShell();
    fireEvent.click(within(screen.getByTestId('adventure-mod')).getByRole('button', { name: 'Start' }));
    fireEvent.click(screen.getByRole('button', { name: 'Exit' }));
    expect(screen.getByText('Agora Lab')).toBeInTheDocument();
    expect(screen.getByTestId('adventure-build')).toBeInTheDocument();
  });
});
