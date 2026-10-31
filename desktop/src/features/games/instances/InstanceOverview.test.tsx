/**
 * The Overview's two choices that must not go wrong silently: a "Launch anyway" tick made for one
 * instance must not carry to another, and a deployment change that was saved must not read as
 * "not changed" because the page could not refresh afterwards.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { InstanceOverviewTab } from './InstanceOverview';
import {
  checkGameInstance,
  getGameInstance,
  setGameInstanceDeployment,
  type InstanceCheck,
  type InstanceDetail,
} from './api';

vi.mock('./api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./api')>();
  return {
    ...actual,
    getGameInstance: vi.fn(),
    checkGameInstance: vi.fn(),
    setGameInstanceDeployment: vi.fn(),
    onInstanceExited: vi.fn(() => Promise.resolve(() => undefined)),
  };
});

function detail(instanceId: string): InstanceDetail {
  return {
    instanceId,
    gameId: 'skyrim-se',
    gameName: 'Skyrim Special Edition',
    name: instanceId === 'grounded-1' ? 'Grounded' : 'Modded',
    pinned: true,
    store: 'steam',
    version: '1.6.1170',
    build: null,
    baseId: 'skyrim-se_steam_1.6.1170',
    installId: null,
    unpinnedReason: null,
    deployment: null,
    saves: 'shared',
    createdAt: '2026-10-01 10:00:00',
    lastLaunchedAt: null,
    running: false,
  };
}

const cleanCheck: InstanceCheck = { instanceId: 'grounded-1', checked: true, status: 'ok', findings: [] };

beforeEach(() => {
  vi.mocked(getGameInstance).mockReset();
  vi.mocked(checkGameInstance).mockReset().mockResolvedValue(cleanCheck);
  vi.mocked(setGameInstanceDeployment).mockReset();
});

describe('Launch anyway', () => {
  it('does not carry a tick from one instance over to the next', async () => {
    vi.mocked(getGameInstance).mockImplementation(async (id) => detail(id));
    const { rerender } = render(<InstanceOverviewTab gameId="skyrim-se" instanceId="grounded-1" />);
    await screen.findByTestId('instance-overview');

    fireEvent.click(screen.getByRole('button', { name: 'Advanced options' }));
    fireEvent.click(screen.getByRole('checkbox', { name: /Launch anyway/ }));
    expect(screen.getByRole('checkbox', { name: /Launch anyway/ })).toBeChecked();

    rerender(<InstanceOverviewTab gameId="skyrim-se" instanceId="modded-2" />);
    await screen.findByRole('heading', { name: 'Modded' });
    expect(screen.getByRole('checkbox', { name: /Launch anyway/ })).not.toBeChecked();
  });
});

describe('deployment change', () => {
  it('says the change was saved when only the refresh afterwards fails', async () => {
    let loads = 0;
    vi.mocked(getGameInstance).mockImplementation(async (id) => {
      loads += 1;
      if (loads > 1) throw new Error('the page could not refresh');
      return detail(id);
    });
    vi.mocked(setGameInstanceDeployment).mockResolvedValue({ instanceId: 'grounded-1', deployment: 'links' });
    render(<InstanceOverviewTab gameId="skyrim-se" instanceId="grounded-1" />);
    await screen.findByTestId('instance-overview');

    fireEvent.click(screen.getByRole('button', { name: 'Linked files' }));

    await waitFor(() => expect(setGameInstanceDeployment).toHaveBeenCalledWith('grounded-1', 'links'));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('the page could not refresh');
    expect(alert).not.toHaveTextContent('was not changed');
  });

  it('says it was not changed when the save itself fails', async () => {
    vi.mocked(getGameInstance).mockImplementation(async (id) => detail(id));
    vi.mocked(setGameInstanceDeployment).mockRejectedValue(new Error('the save was refused'));
    render(<InstanceOverviewTab gameId="skyrim-se" instanceId="grounded-1" />);
    await screen.findByTestId('instance-overview');

    fireEvent.click(screen.getByRole('button', { name: 'Linked files' }));

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('The way this instance runs was not changed. the save was refused');
  });
});
