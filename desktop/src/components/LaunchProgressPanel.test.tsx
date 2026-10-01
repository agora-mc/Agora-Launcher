import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { LaunchProgressPanel } from './LaunchProgressPanel';
import type { LaunchProgressInfo } from '@/lib/launchProgress';

const base: LaunchProgressInfo = { stage: 'starting', label: 'Starting', startedAt: Date.now(), files: null };

describe('LaunchProgressPanel', () => {
  it('shows an indeterminate bar while counts are unknown', () => {
    render(<LaunchProgressPanel progress={base} />);
    const bar = screen.getByRole('progressbar');
    expect(bar.getAttribute('aria-valuenow')).toBeNull();
    expect(screen.getByText('Starting')).toBeTruthy();
  });

  it('shows a determinate bar and counts during the files stage', () => {
    render(
      <LaunchProgressPanel
        progress={{ ...base, stage: 'files', label: 'Downloading and verifying game files', files: { kind: 'libraries', done: 20, total: 80 } }}
      />,
    );
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('25');
    expect(screen.getByText(/libraries 20 \/ 80/)).toBeTruthy();
  });

  it('shows the Java download percentage and status', () => {
    render(
      <LaunchProgressPanel
        progress={{ ...base, stage: 'java', label: 'Downloading Java', percent: 40, detail: 'Downloading Java 21: 20 of 52 MB' }}
      />,
    );
    expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBe('40');
    expect(screen.getByText(/20 of 52 MB/)).toBeTruthy();
  });

  it('says plainly that loading is unconfirmed, and opens the Console', () => {
    const onOpenConsole = vi.fn();
    render(<LaunchProgressPanel progress={{ ...base, stage: 'loading', label: 'Running — loading' }} onOpenConsole={onOpenConsole} />);
    expect(screen.getByText('Running — loading')).toBeTruthy();
    expect(screen.getByText(/still loading/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Open Console' }));
    expect(onOpenConsole).toHaveBeenCalled();
  });

  it('shows Ready without a bar or elapsed timer', () => {
    render(<LaunchProgressPanel progress={{ ...base, stage: 'ready', label: 'Ready' }} />);
    expect(screen.getByText('Ready')).toBeTruthy();
    expect(screen.queryByRole('progressbar')).toBeNull();
    expect(screen.queryByLabelText('Elapsed time')).toBeNull();
  });
});
