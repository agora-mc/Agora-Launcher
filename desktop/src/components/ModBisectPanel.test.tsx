import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { ModBisectPanel } from './ModBisectPanel';
import type { BisectView } from '@/lib/tauri';

const mocks = vi.hoisted(() => ({
  getBisectSession: vi.fn(),
  startBisect: vi.fn(),
  applyBisectTrial: vi.fn(),
  recordBisectOutcome: vi.fn(),
  stepBackBisect: vi.fn(),
  cancelBisect: vi.fn(),
}));

vi.mock('@/lib/tauri', () => ({
  ...mocks,
  formatError: (e: unknown) => String(e),
}));

function view(overrides: Partial<BisectView> = {}): BisectView {
  return {
    session: {
      schema_version: 1,
      started_at: '2026-09-01T00:00:00Z',
      baseline_enabled: ['a.jar', 'b.jar', 'c.jar', 'd.jar'],
      suspects: ['a.jar', 'b.jar', 'c.jar', 'd.jar'],
      history: [],
      invert_next_split: false,
    },
    trial: {
      status: { type: 'awaiting_trial' },
      enable: ['a.jar', 'b.jar'],
      disable: ['c.jar', 'd.jar'],
      completed_trials: 0,
      remaining_trials: 2,
    },
    ...overrides,
  };
}

describe('ModBisectPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.getBisectSession.mockResolvedValue({ session: null, trial: null });
  });

  it('offers to start when no bisect is running', async () => {
    render(<ModBisectPanel instanceId="test" />);
    expect(await screen.findByRole('button', { name: 'Start' })).toBeInTheDocument();
  });

  it('passes crash-named suspects through so they are tested first', async () => {
    mocks.startBisect.mockResolvedValue(view());
    render(<ModBisectPanel instanceId="test" primeSuspects={['sodium.jar']} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Start' }));
    await waitFor(() => {
      expect(mocks.startBisect).toHaveBeenCalledWith('test', ['sodium.jar']);
    });
  });

  it('reports progress and records which way the trial went', async () => {
    mocks.getBisectSession.mockResolvedValue(view());
    mocks.recordBisectOutcome.mockResolvedValue(view());
    render(<ModBisectPanel instanceId="test" />);
    expect(await screen.findByText(/Trial 1/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Still broken' }));
    await waitFor(() => {
      expect(mocks.recordBisectOutcome).toHaveBeenCalledWith('test', true);
    });

    fireEvent.click(screen.getByRole('button', { name: 'Worked fine' }));
    await waitFor(() => {
      expect(mocks.recordBisectOutcome).toHaveBeenCalledWith('test', false);
    });
  });

  it('cannot step back before the first answer', async () => {
    mocks.getBisectSession.mockResolvedValue(view());
    render(<ModBisectPanel instanceId="test" />);
    expect(await screen.findByRole('button', { name: 'Step back' })).toBeDisabled();
  });

  it('names the culprit once the search converges', async () => {
    mocks.getBisectSession.mockResolvedValue(
      view({ trial: { status: { type: 'culprit', filename: 'sodium.jar' }, enable: [], disable: [], completed_trials: 3, remaining_trials: 0 } }),
    );
    render(<ModBisectPanel instanceId="test" />);
    expect(await screen.findByText('sodium.jar')).toBeInTheDocument();
  });

  it('shows the whole group when the dependency graph blocks a finer answer', async () => {
    mocks.getBisectSession.mockResolvedValue(
      view({ trial: { status: { type: 'culprit_group', filenames: ['a.jar', 'b.jar'] }, enable: [], disable: [], completed_trials: 2, remaining_trials: 0 } }),
    );
    render(<ModBisectPanel instanceId="test" />);
    expect(await screen.findByText('a.jar')).toBeInTheDocument();
    expect(screen.getByText('b.jar')).toBeInTheDocument();
  });

  it('says so plainly when no single mod is to blame', async () => {
    mocks.getBisectSession.mockResolvedValue(
      view({ trial: { status: { type: 'inconclusive' }, enable: [], disable: [], completed_trials: 4, remaining_trials: 0 } }),
    );
    render(<ModBisectPanel instanceId="test" />);
    expect(await screen.findByText(/not one mod on its own/)).toBeInTheDocument();
  });

  it('does not restore anything unless the user confirms', async () => {
    mocks.getBisectSession.mockResolvedValue(view());
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);
    render(<ModBisectPanel instanceId="test" />);
    fireEvent.click(await screen.findByRole('button', { name: /Stop and restore/ }));
    expect(mocks.cancelBisect).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });
});
