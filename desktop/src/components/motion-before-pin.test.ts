import { beforeEach, describe, expect, it } from 'vitest';
import {
  MOTION_BEFORE_PIN_KEY,
  motionAfterPinEnds,
  rememberMotionBeforePin,
  takeMotionBeforePin,
} from './motion-before-pin';

describe('motion before Simple mode', () => {
  beforeEach(() => window.localStorage.clear());

  it('remembers the first value only and forgets it once taken', () => {
    rememberMotionBeforePin('full');
    rememberMotionBeforePin('reduced');
    expect(window.localStorage.getItem(MOTION_BEFORE_PIN_KEY)).toBe('full');
    expect(takeMotionBeforePin()).toBe('full');
    expect(takeMotionBeforePin()).toBeNull();
  });

  it('ignores a corrupt saved value', () => {
    window.localStorage.setItem(MOTION_BEFORE_PIN_KEY, 'warp');
    expect(takeMotionBeforePin()).toBeNull();
  });

  it('restores what Simple changed, but keeps a value the user set since', () => {
    expect(motionAfterPinEnds('reduced', 'reduced', 'full')).toBe('full');
    expect(motionAfterPinEnds('reduced', 'reduced', null)).toBe('system');
    expect(motionAfterPinEnds('full', 'reduced', 'system')).toBe('full');
  });
});
