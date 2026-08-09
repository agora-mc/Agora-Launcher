import { beforeEach, describe, expect, it } from 'vitest';
import {
  clearProgress,
  loadAdventureProgress,
  loadProgress,
  recordCheckpoint,
} from './progressStore';

beforeEach(() => {
  window.localStorage.clear();
});

describe('progressStore', () => {
  it('starts empty', () => {
    expect(loadProgress()).toEqual({ version: 1, adventures: {} });
    expect(loadAdventureProgress('build', 1)).toBeNull();
  });

  it('records and resumes checkpoints', () => {
    recordCheckpoint('build', 1, 1, 1, false);
    const record = loadAdventureProgress('build', 1);
    expect(record).toMatchObject({ lessonVersion: 1, completedCheckpoints: 1, lastSafeCheckpoint: 1, completed: false });
  });

  it('records completion without overwriting earlier progress', () => {
    recordCheckpoint('build', 1, 1, 1, false);
    recordCheckpoint('build', 1, 3, 3, true);
    const record = loadAdventureProgress('build', 1);
    expect(record?.completed).toBe(true);
    expect(record?.completedCheckpoints).toBe(3);
  });

  it('treats a lesson version mismatch as empty (reset safely)', () => {
    recordCheckpoint('build', 1, 2, 2, false);
    expect(loadAdventureProgress('build', 1)).not.toBeNull();
    expect(loadAdventureProgress('build', 2)).toBeNull();
  });

  it('resets safely on schema mismatch', () => {
    window.localStorage.setItem('agora-lab-progress', JSON.stringify({ version: 99, adventures: {} }));
    expect(loadProgress()).toEqual({ version: 1, adventures: {} });
    window.localStorage.setItem('agora-lab-progress', '{ not json');
    expect(loadProgress()).toEqual({ version: 1, adventures: {} });
  });

  it('stores only non-sensitive metadata', () => {
    recordCheckpoint('build', 1, 2, 2, true);
    const raw = window.localStorage.getItem('agora-lab-progress') ?? '';
    expect(raw).not.toContain('path');
    expect(raw).not.toContain('instance');
    expect(raw).not.toContain('token');
    expect(raw).not.toContain('C:\\');
  });

  it('clears all progress', () => {
    recordCheckpoint('build', 1, 1, 1, false);
    clearProgress();
    expect(loadProgress()).toEqual({ version: 1, adventures: {} });
  });
});
