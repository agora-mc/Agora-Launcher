import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { LaunchFailedBanner } from './LaunchFailedBanner';

describe('launch failed banner', () => {
  it('names the Crash Doctor action for what it does, and opens it', () => {
    const onInvestigate = vi.fn();
    render(
      <LaunchFailedBanner exitCode={1} onOpenConsole={() => undefined} onInvestigate={onInvestigate} />,
    );
    // "Find problems" suggested a readiness check; Crash Doctor only reads a crashed game.
    expect(screen.queryByRole('button', { name: 'Find problems' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Diagnose a crash' }));
    expect(onInvestigate).toHaveBeenCalledTimes(1);
  });
});
