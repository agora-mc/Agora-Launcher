import { describe, expect, it, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { HealthDialog } from './HealthDialog';
import type { HealthReport } from '@/lib/tauri';

const tauriMocks = vi.hoisted(() => ({
  disableModForTest: vi.fn(async () => undefined),
  checkInstanceHealth: vi.fn(async () => ({})),
}));

vi.mock('@/lib/tauri', () => ({
  disableModForTest: tauriMocks.disableModForTest,
  checkInstanceHealth: tauriMocks.checkInstanceHealth,
}));

vi.mock('@/lib/healthPreferences', () => ({
  loadHealthPreferences: async () => ({ mutedWarnings: [], muteAllRecommendations: false }),
  saveHealthPreferences: async () => undefined,
  activeHealthWarnings: (warnings: unknown[]) => warnings,
  healthWarningKey: (warning: { kind: string; filename?: string | null }) => `${warning.kind}:${warning.filename ?? ''}`,
}));

function report(): HealthReport {
  return {
    score: 'red',
    scan_token: 't',
    blockers: [
      {
        kind: 'incompatible_mod',
        mod_id: 'sodium',
        filename: 'sodium.jar',
        message: 'Sodium is incompatible with this loader.',
        suggested_action: null,
      },
    ],
    warnings: [],
    recommendations: [],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  tauriMocks.checkInstanceHealth.mockResolvedValue(report());
});

function renderDialog(reviewOnly: boolean) {
  return render(
    <HealthDialog
      instanceId="inst-1"
      instanceName="My World"
      initialReport={report()}
      onConfirm={async () => null}
      onCancel={() => undefined}
      onSwitchLoader={async () => undefined}
      reviewOnly={reviewOnly}
    />,
  );
}

describe('HealthDialog review-only safety (SOL-2 §18.5)', () => {
  it('reviewOnly hides the rejected direct-disable control and never calls disableModForTest', async () => {
    renderDialog(true);
    await screen.findByText(/incompatible with this loader/i);
    // The approved review surface keeps inspection only: no Disable repair.
    expect(screen.queryByRole('button', { name: 'Disable' })).not.toBeInTheDocument();
    expect(tauriMocks.disableModForTest).not.toHaveBeenCalled();
  });

  it('non-review (Standard) mode keeps the existing disable repair', async () => {
    renderDialog(false);
    await screen.findByRole('button', { name: 'Disable' });
    fireEvent.click(screen.getByRole('button', { name: 'Disable' }));
    await waitFor(() => expect(tauriMocks.disableModForTest).toHaveBeenCalledWith('inst-1', 'sodium.jar'));
  });
});
