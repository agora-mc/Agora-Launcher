import { describe, expect, it, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { WhyInstalledDialog } from './WhyInstalledDialog';
import type { PresenceExplanation } from '@/lib/tauri';

const tauriMocks = vi.hoisted(() => ({
  explainModPresence: vi.fn(),
}));

vi.mock('@/lib/tauri', () => ({
  explainModPresence: tauriMocks.explainModPresence,
  formatError: (error: unknown) => String(error),
}));

function explanation(overrides: Partial<PresenceExplanation> = {}): PresenceExplanation {
  return {
    filename: 'corelib.jar',
    installed_as_dependency: true,
    pack_managed: false,
    dependents: [],
    root_paths: [],
    orphaned: false,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('WhyInstalledDialog', () => {
  it('renders the chain from the mod the user actually asked for', async () => {
    tauriMocks.explainModPresence.mockResolvedValue(
      explanation({ root_paths: [['caves.jar', 'corelib.jar', 'base.jar']] }),
    );
    render(
      <WhyInstalledDialog
        instanceId="alpha"
        filename="base.jar"
        displayName="Base Lib"
        onClose={() => {}}
      />,
    );
    await waitFor(() => expect(screen.getByText('Required by')).toBeInTheDocument());
    expect(screen.getByText('caves.jar')).toBeInTheDocument();
    expect(screen.getByText('corelib.jar')).toBeInTheDocument();
    // The target also appears in the dialog subtitle, so scope to the chain and
    // check it is rendered as the emphasised final step.
    expect(screen.getByRole('listitem')).toHaveTextContent('caves.jar → corelib.jar → base.jar');
  });

  it('says plainly when the user installed it themselves', async () => {
    tauriMocks.explainModPresence.mockResolvedValue(
      explanation({ installed_as_dependency: false }),
    );
    render(
      <WhyInstalledDialog
        instanceId="alpha"
        filename="sodium.jar"
        displayName="Sodium"
        onClose={() => {}}
      />,
    );
    await waitFor(() => expect(screen.getByText('You installed this directly.')).toBeInTheDocument());
  });

  it('credits the modpack ahead of the dependency reason', async () => {
    // A pack mod can also be flagged as a dependency; the pack is the answer
    // the user needs, so it has to win.
    tauriMocks.explainModPresence.mockResolvedValue(
      explanation({ pack_managed: true, installed_as_dependency: true }),
    );
    render(
      <WhyInstalledDialog
        instanceId="alpha"
        filename="corelib.jar"
        displayName="Core Lib"
        onClose={() => {}}
      />,
    );
    await waitFor(() => expect(
      screen.getByText('This came from the instance’s modpack.'),
    ).toBeInTheDocument());
  });

  it('flags an orphan so the user knows it is safe to drop', async () => {
    tauriMocks.explainModPresence.mockResolvedValue(explanation({ orphaned: true }));
    render(
      <WhyInstalledDialog
        instanceId="alpha"
        filename="corelib.jar"
        displayName="Core Lib"
        onClose={() => {}}
      />,
    );
    await waitFor(() => expect(
      screen.getByText('Agora installed this as a dependency, and nothing needs it any more.'),
    ).toBeInTheDocument());
    expect(screen.getByText('Nothing installed declares a dependency on it.')).toBeInTheDocument();
  });

  it('surfaces a lookup failure instead of an empty dialog', async () => {
    tauriMocks.explainModPresence.mockRejectedValue(new Error('no manifest'));
    render(
      <WhyInstalledDialog
        instanceId="alpha"
        filename="corelib.jar"
        displayName="Core Lib"
        onClose={() => {}}
      />,
    );
    await waitFor(() => expect(screen.getByText(/no manifest/)).toBeInTheDocument());
  });
});
