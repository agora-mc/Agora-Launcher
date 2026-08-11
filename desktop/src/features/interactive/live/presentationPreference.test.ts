import { beforeEach, describe, expect, it } from 'vitest';
import {
  clearPreference,
  effectiveView,
  isHighInteractionSuspended,
  loadPreference,
  resumeHighInteractionView,
  savePreference,
  suspendHighInteraction,
} from './presentationPreference';

beforeEach(() => {
  window.localStorage.clear();
});

describe('interaction presentation preference', () => {
  it('defaults to standard (safe) with no stored value', () => {
    expect(loadPreference()).toBe('standard');
  });

  it('persists high-interaction and reloads it', () => {
    savePreference('high-interaction');
    expect(loadPreference()).toBe('high-interaction');
  });

  it('resets safely on schema/version mismatch', () => {
    window.localStorage.setItem('agora-interaction-preference', JSON.stringify({ version: 99, value: 'high-interaction' }));
    expect(loadPreference()).toBe('standard');
    window.localStorage.setItem('agora-interaction-preference', '{ not json');
    expect(loadPreference()).toBe('standard');
    window.localStorage.setItem('agora-interaction-preference', JSON.stringify({ version: 1, value: 'bogus' }));
    expect(loadPreference()).toBe('standard');
  });

  it('stores only the non-sensitive preference', () => {
    savePreference('high-interaction');
    const raw = window.localStorage.getItem('agora-interaction-preference') ?? '';
    expect(raw).not.toMatch(/instance|path|token|sha/);
  });

  it('clearPreference resets to standard', () => {
    savePreference('high-interaction');
    expect(clearPreference()).toBe('standard');
    expect(loadPreference()).toBe('standard');
  });
});

/**
 * SOL §22.4 — session view state vs persisted preference.
 *
 * §18.4 requires every bridge to LEAVE High Interaction before Standard work so
 * the live host unmounts. That must not destroy the user's saved choice, which
 * is what made the mode silently turn itself off after any review (T6-4).
 */
describe('High Interaction: session view vs persisted preference (SOL §22.4)', () => {
  beforeEach(() => {
    window.localStorage.clear();
    resumeHighInteractionView();
  });

  it('a bridge suspending the view does NOT rewrite the saved preference', () => {
    savePreference('high-interaction');
    suspendHighInteraction();
    // §18.4: the host must not render right now.
    expect(effectiveView()).toBe('standard');
    expect(isHighInteractionSuspended()).toBe(true);
    // ...but the user's choice survives.
    expect(loadPreference()).toBe('high-interaction');
  });

  it('the preference is honoured again once the session resumes', () => {
    savePreference('high-interaction');
    suspendHighInteraction();
    resumeHighInteractionView();
    expect(effectiveView()).toBe('high-interaction');
  });

  it('an explicit switch to Standard persists, and is not undone by resuming', () => {
    savePreference('high-interaction');
    savePreference('standard');
    resumeHighInteractionView();
    expect(effectiveView()).toBe('standard');
    expect(loadPreference()).toBe('standard');
  });

  it('defaults to standard when nothing was ever chosen', () => {
    expect(effectiveView()).toBe('standard');
  });
});
