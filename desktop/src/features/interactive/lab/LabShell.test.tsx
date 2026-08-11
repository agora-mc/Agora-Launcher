import { beforeEach, describe, expect, it } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
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

/** Drive Mod It to checkpoint 2 (conflict visible, unresolved). */
async function modItToConflict(user: ReturnType<typeof userEvent.setup>) {
  await user.click(within(screen.getByTestId('adventure-mod')).getByRole('button', { name: /^Start / }));
  await user.click(screen.getByRole('button', { name: 'Stage BetterCaves' }));
  await user.click(screen.getByRole('button', { name: 'Snap required: Core Lib' }));
  await user.click(screen.getByRole('button', { name: 'Include optional: Nice Textures' }));
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
    fireEvent.click(within(buildCard).getByRole('button', { name: /^Start / }));

    expect(screen.getAllByText('Simulation').length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText(/Step 1 of 3/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Place Minecraft 1.20.1' }));
    expect(screen.getByText('Minecraft 1.20.1 is placed on the bench.')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Choose Forge' }));
    expect(screen.getAllByText(/Forge needs Minecraft 1\.21/).length).toBeGreaterThanOrEqual(1);
    // T6-3: the rejected loader must NOT become current state. It renders as a
    // proposal marked incompatible, and the bench keeps its previous loader.
    expect(screen.getAllByText('Incompatible').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('proposed loader choice').length).toBeGreaterThanOrEqual(1);

    fireEvent.click(screen.getByRole('button', { name: 'Choose Fabric' }));
    // Choosing a compatible loader clears the rejected proposal entirely.
    expect(screen.queryByText('proposed loader choice')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Name it "My Redstone World"' }));
    fireEvent.click(screen.getByRole('button', { name: 'Place Notebot Mod' }));

    expect(screen.getByText('Adventure complete')).toBeInTheDocument();
    expect(screen.getAllByText(/You completed the practice/).length).toBeGreaterThanOrEqual(1);
    expect(screen.getByRole('button', { name: /Field Guide: Instances/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'My Instances' })).toBeInTheDocument();
  });

  it('Reset restores the first checkpoint', () => {
    renderShell();
    fireEvent.click(within(screen.getByTestId('adventure-build')).getByRole('button', { name: /^Start / }));
    fireEvent.click(screen.getByRole('button', { name: 'Place Minecraft 1.20.1' }));
    fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
    expect(screen.getByText(/Step 1 of 3/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Place Minecraft 1.20.1' })).toBeInTheDocument();
  });

  it('Undo It requires a serious confirmation before restore', () => {
    renderShell();
    fireEvent.click(within(screen.getByTestId('adventure-undo')).getByRole('button', { name: /^Start / }));

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
    fireEvent.click(within(screen.getByTestId('adventure-undo')).getByRole('button', { name: /^Start / }));
    fireEvent.click(screen.getByRole('button', { name: 'Compare Last known good' }));
    fireEvent.click(screen.getByRole('button', { name: 'Restore Weekend manual snapshot (worlds included)' }));
    fireEvent.click(screen.getByRole('button', { name: 'Try restore now' }));

    expect(screen.getAllByText(/Blocked: the instance is running/).length).toBeGreaterThanOrEqual(1);
    expect(screen.getByRole('button', { name: 'Try restore now' }).className).toContain('destructive');
  });

  it('Mod It: replacement confirmation is action-specific and post-apply state is fully current', () => {
    renderShell();
    fireEvent.click(within(screen.getByTestId('adventure-mod')).getByRole('button', { name: /^Start / }));

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
    fireEvent.click(within(screen.getByTestId('adventure-mod')).getByRole('button', { name: /^Start / }));
    fireEvent.click(screen.getByRole('button', { name: 'Exit' }));
    expect(screen.getByText('Agora Lab')).toBeInTheDocument();
    expect(screen.getByTestId('adventure-build')).toBeInTheDocument();
  });

  it('routes the graph Stage removal gesture through the same confirmation gate (BLOCKER 2)', async () => {
    const user = userEvent.setup();
    renderShell();
    await modItToConflict(user);

    // Alternate visual route: the graph's Stage removal control on Terrain Overhaul.
    const stageRemoval = screen.getByRole('button', { name: /^Stage removal:/ });
    await user.click(stageRemoval);

    // Confirmation opens and the proposal is NOT yet changed.
    expect(screen.getByRole('alertdialog', { name: 'Confirm replacement' })).toBeInTheDocument();
    expect(screen.queryByText('Proposed: remove')).not.toBeInTheDocument();

    // Cancelling leaves current and proposed state unchanged.
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
    expect(screen.queryByText('Proposed: remove')).not.toBeInTheDocument();

    // Confirming follows the same result as the action-list route.
    await user.click(stageRemoval);
    await user.click(screen.getByRole('button', { name: 'Confirm' }));
    expect(screen.getByText('Proposed: remove')).toBeInTheDocument();
    expect(screen.getByText(/marked for removal/)).toBeInTheDocument();
  });

  it('confirmation is exclusive: the background is inert while open (BLOCKER 3)', async () => {
    const user = userEvent.setup();
    renderShell();
    await modItToConflict(user);

    await user.click(screen.getByRole('button', { name: 'Replace Terrain Overhaul' }));
    expect(screen.getByRole('alertdialog', { name: 'Confirm replacement' })).toBeInTheDocument();

    // Modal background exclusion: the underlying decision control is removed
    // from the accessibility tree while the confirmation is open.
    expect(screen.queryByRole('button', { name: 'Keep current content' })).not.toBeInTheDocument();

    // Escape cancels the confirmation.
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
    expect(screen.queryByText('Proposed: remove')).not.toBeInTheDocument();
    // The background becomes accessible again.
    expect(screen.getByRole('button', { name: 'Keep current content' })).toBeInTheDocument();
  });

  it('confirmation moves focus to Cancel and restores focus to the origin (BLOCKER 3)', async () => {
    const user = userEvent.setup();
    renderShell();
    await modItToConflict(user);

    const replace = screen.getByRole('button', { name: 'Replace Terrain Overhaul' });
    await user.click(replace);

    // Initial focus lands on the safe Cancel target.
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();

    // Escape cancels.
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();

    // Focus returns to the invoking control.
    expect(replace).toHaveFocus();
  });

  it('keeps focus contained within the confirmation dialog (residual BLOCKER B)', async () => {
    const user = userEvent.setup();
    renderShell();
    await modItToConflict(user);

    await user.click(screen.getByRole('button', { name: 'Replace Terrain Overhaul' }));
    const dialog = screen.getByRole('alertdialog', { name: 'Confirm replacement' });
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();

    // Tab forward: focus stays inside the dialog on every step.
    for (let i = 0; i < 8; i += 1) {
      await user.keyboard('{Tab}');
      expect(dialog.contains(document.activeElement)).toBe(true);
    }
    // Shift+Tab back: focus stays inside the dialog on every step.
    for (let i = 0; i < 8; i += 1) {
      await user.keyboard('{Shift>}{Tab}{/Shift}');
      expect(dialog.contains(document.activeElement)).toBe(true);
    }
  });

  it('plays Heal It end-to-end: scan, fix loader, review warning, revalidate', async () => {
    const user = userEvent.setup();
    renderShell();
    await user.click(within(screen.getByTestId('adventure-heal')).getByRole('button', { name: /^Start / }));

    await user.click(screen.getByRole('button', { name: 'Run validation check' }));
    expect(screen.getByText(/1 blocker, 1 warning, 1 recommendation/)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Choose Fabric 0.15' }));
    expect(screen.getByText(/the blocker is cleared/)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Let Agora manage Java 17' }));
    await user.click(screen.getByRole('button', { name: 'Keep automatic memory' }));

    await user.click(screen.getByRole('button', { name: 'Re-run validation check' }));
    expect(screen.getByText('Adventure complete')).toBeInTheDocument();
    expect(screen.getAllByText(/no blockers/).length).toBeGreaterThanOrEqual(1);
  });

  it('plays Fix It end-to-end: evidence, hypothesis, recoverable experiment, confirm', async () => {
    const user = userEvent.setup();
    renderShell();
    await user.click(within(screen.getByTestId('adventure-fix')).getByRole('button', { name: /^Start / }));

    await user.click(screen.getByRole('button', { name: 'Read the clues' }));
    await user.click(screen.getByRole('button', { name: 'Test: a mod fails during startup' }));

    // The experiment is a serious decision: confirm dialog, not immediate dispatch.
    await user.click(screen.getByRole('button', { name: 'Create recovery point, then test one change' }));
    expect(screen.getByRole('alertdialog', { name: 'Confirm experiment' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Confirm' }));

    expect(screen.getByText(/one launch supports the hypothesis/i)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Confirm: crash is gone' }));
    expect(screen.getByText('Adventure complete')).toBeInTheDocument();
    expect(screen.getByText('Recovery confirmed')).toBeInTheDocument();
  });

  it('plays Take It Offline end-to-end: inspect, prepare, delegated, recheck', async () => {
    const user = userEvent.setup();
    renderShell();
    await user.click(within(screen.getByTestId('adventure-offline')).getByRole('button', { name: /^Start / }));

    await user.click(screen.getByRole('button', { name: 'Inspect readiness for delegated launch' }));
    expect(screen.getByText(/One content file is not downloaded yet/)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Download the missing file now' }));
    await user.click(screen.getByRole('button', { name: 'Use delegated launch' }));

    // The map's re-check control and the shell decision share the label; either route completes.
    await user.click(screen.getAllByRole('button', { name: 'Re-check readiness' })[0]);
    expect(screen.getByText('Adventure complete')).toBeInTheDocument();
    expect(screen.getByText(/cached catalog alone never makes an instance ready/)).toBeInTheDocument();
  });
});
