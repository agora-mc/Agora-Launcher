/**
 * Home's "Continue" card must know about instance health, the same way My
 * Instances does: a blocked instance is "not ready" and its main action is
 * "See what's wrong"; a healthy one still offers Play. Home gets the reports
 * from the app-level health monitor, so opening Home must not start a scan.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { Home } from './Home';
import type { HealthReport } from '../lib/tauri';
import type { ProcessState } from '../lib/useProcessController';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

const row = {
  instance_id: 'alpha',
  name: 'Alpha',
  loader: 'fabric',
  loader_version: '0.19.5',
  minecraft_version: '26.2',
  is_locked: false,
  last_launched_at: '2026-10-01T00:00:00Z',
  launch_mode_override: null,
  import_source: null,
  icon_path: null,
};

function report(overrides: Partial<HealthReport> = {}): HealthReport {
  return {
    score: 'green',
    blockers: [],
    warnings: [],
    recommendations: [],
    scan_token: 'home-test',
    ...overrides,
  };
}

const blocker = {
  kind: 'missing_required_dependency',
  mod_id: 'fabric-api',
  filename: 'example.jar',
  message: 'example needs fabric-api, which is not installed.',
  suggested_action: null,
};

const warning = {
  kind: 'unknown_mod',
  mod_id: 'example',
  filename: 'example.jar',
  message: 'Example warning',
  suggested_action: null,
};

const idle: ProcessState = {
  phase: 'idle',
  instanceId: null,
  pid: null,
  error: null,
  errorCode: null,
  healthReport: null,
  directLaunch: false,
  exitCode: null,
  outcome: null,
  snapshotId: null,
  exitedAt: null,
} as ProcessState;

function installInvoke() {
  invokeMock.mockImplementation((command: string) => {
    if (command === 'list_instances') return Promise.resolve([row]);
    if (command === 'get_setting') return Promise.resolve(false);
    return Promise.resolve(null);
  });
}

function renderHome(healthReports: Record<string, HealthReport>, onReviewHealth = vi.fn(), onLaunch = vi.fn(async () => true)) {
  render(
    <Home
      onNavigateTab={vi.fn()}
      onOpenInstance={vi.fn()}
      onOpenMod={vi.fn()}
      onLaunch={onLaunch}
      processState={idle}
      onKillProcess={vi.fn(async () => undefined)}
      healthReports={healthReports}
      onReviewHealth={onReviewHealth}
    />,
  );
  return { onReviewHealth, onLaunch };
}

function calls(command: string) {
  return invokeMock.mock.calls.filter(([name]) => name === command).length;
}

describe('Home continue card and instance health', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    installInvoke();
  });

  it('shows the not-ready state and "See what\'s wrong" for a blocked instance', async () => {
    const blocked = report({ score: 'red', blockers: [blocker] });
    renderHome({ alpha: blocked });

    expect(await screen.findByRole('button', { name: "See what's wrong" })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Play Alpha' })).not.toBeInTheDocument();
    expect(screen.getByText('1 mod problem to fix')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Launch anyway' })).toBeInTheDocument();
  });

  it('opens the health review for that instance when "See what\'s wrong" is clicked', async () => {
    const blocked = report({ score: 'red', blockers: [blocker] });
    const { onReviewHealth } = renderHome({ alpha: blocked });

    fireEvent.click(await screen.findByRole('button', { name: "See what's wrong" }));

    expect(onReviewHealth).toHaveBeenCalledTimes(1);
    expect(onReviewHealth).toHaveBeenCalledWith('alpha', 'Alpha', blocked);
  });

  it('still launches from "Launch anyway" on a blocked instance', async () => {
    const { onLaunch } = renderHome({ alpha: report({ score: 'red', blockers: [blocker] }) });

    fireEvent.click(await screen.findByRole('button', { name: 'Launch anyway' }));

    await waitFor(() => expect(onLaunch).toHaveBeenCalledWith('alpha', false));
  });

  it('keeps the Play button for a healthy instance', async () => {
    renderHome({ alpha: report() });

    expect(await screen.findByRole('button', { name: 'Play Alpha' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: "See what's wrong" })).not.toBeInTheDocument();
  });

  it('keeps Play for an instance with only warnings, and says so', async () => {
    renderHome({ alpha: report({ score: 'yellow', warnings: [warning] }) });

    expect(await screen.findByRole('button', { name: 'Play Alpha' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: "See what's wrong" })).not.toBeInTheDocument();
    expect(screen.getByText('1 warning to review')).toBeInTheDocument();
  });

  it('shows Play when no health check has run yet', async () => {
    renderHome({});

    expect(await screen.findByRole('button', { name: 'Play Alpha' })).toBeInTheDocument();
  });

  it('reads the reports it is given and starts no health scan on load', async () => {
    renderHome({ alpha: report({ score: 'red', blockers: [blocker] }) });

    await screen.findByRole('button', { name: "See what's wrong" });
    expect(calls('check_instance_health')).toBe(0);
    expect(calls('check_all_instance_health')).toBe(0);
  });
});
