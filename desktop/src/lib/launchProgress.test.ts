import { describe, expect, it } from 'vitest';
import {
  applyLaunchProgressEvent,
  filesDetail,
  filesPercent,
  formatElapsed,
  STARTING_PROGRESS,
  type LaunchProgressInfo,
} from './launchProgress';

const ev = (phase: string, extra: Record<string, unknown> = {}) => ({ instance_id: 'a', phase, ...extra });

describe('applyLaunchProgressEvent', () => {
  it('walks the stages in order and keeps the launch start time', () => {
    let p: LaunchProgressInfo | null = STARTING_PROGRESS(1000);
    p = applyLaunchProgressEvent(p, ev('checking-health'), 2000);
    expect(p).toMatchObject({ stage: 'checking', startedAt: 2000 });
    p = applyLaunchProgressEvent(p, ev('resolving', { message: 'Preparing Java and the fabric mod loader' }), 3000);
    expect(p).toMatchObject({ stage: 'preparing', label: 'Preparing Java and the fabric mod loader', startedAt: 2000 });
    p = applyLaunchProgressEvent(p, ev('materializing', { files: { kind: 'assets', done: 5, total: 10 } }), 4000);
    expect(p?.stage).toBe('files');
    expect(filesPercent(p!.files)).toBe(50);
    p = applyLaunchProgressEvent(p, ev('launching'), 5000);
    expect(p).toMatchObject({ stage: 'starting', files: null });
    p = applyLaunchProgressEvent(p, ev('running'), 6000);
    expect(p).toMatchObject({ stage: 'loading', label: 'Running — loading' });
    p = applyLaunchProgressEvent(p, ev('ready'), 7000);
    expect(p).toMatchObject({ stage: 'ready', label: 'Ready', startedAt: 2000 });
  });

  it('ignores completion and unknown events', () => {
    const p = STARTING_PROGRESS(1);
    expect(applyLaunchProgressEvent(p, ev('resolving-complete'), 2)).toBe(p);
    expect(applyLaunchProgressEvent(p, ev('mystery'), 2)).toBe(p);
  });
});

describe('file progress helpers', () => {
  it('formats counts and percent, null when unknown', () => {
    expect(filesDetail({ kind: 'libraries', done: 12, total: 80 })).toBe('libraries 12 / 80');
    expect(filesDetail({ kind: 'assets', done: 0, total: 0 })).toBeNull();
    expect(filesPercent(null)).toBeNull();
    expect(filesPercent({ kind: 'assets', done: 99, total: 50 })).toBe(100);
  });
  it('formats elapsed time', () => {
    expect(formatElapsed(4200)).toBe('4s');
    expect(formatElapsed(65000)).toBe('1m 05s');
  });
});
