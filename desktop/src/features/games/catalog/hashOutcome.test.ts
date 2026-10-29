import { describe, expect, it } from 'vitest';
import type { CatalogHash } from './api';
import { hashOutcome } from './hashOutcome';

const SHA = 'a'.repeat(64);

function hash(overrides: Partial<CatalogHash> = {}): CatalogHash {
  return { sha256: SHA, verified: true, basis: 'the source published this hash', confirmedPast: null, ...overrides };
}

describe('hashOutcome', () => {
  it('calls a file verified only when the core verified it', () => {
    const outcome = hashOutcome(hash({ verified: true }));
    expect(outcome.tone).toBe('verified');
    expect(outcome.headline).toMatch(/^Verified/);
    expect(outcome.basis).toBe('the source published this hash');
  });

  it('never calls an unverified file verified, and keeps the core’s reason', () => {
    const outcome = hashOutcome(hash({ verified: false, basis: 'no published hash; told and remembered' }));
    expect(outcome.tone).toBe('unverified');
    expect(outcome.headline).toMatch(/^Not verified/);
    expect(outcome.headline).not.toMatch(/^Verified/);
    expect(outcome.basis).toBe('no published hash; told and remembered');
  });

  it('says which expectation was accepted on purpose, by origin', () => {
    expect(hashOutcome(hash({ confirmedPast: 'curator_pin' })).acceptedPast).toContain('pinned hash');
    expect(hashOutcome(hash({ confirmedPast: 'previous_install' })).acceptedPast).toContain('installed before');
  });

  it('reports no accepted difference when there was none', () => {
    expect(hashOutcome(hash({ confirmedPast: null })).acceptedPast).toBeNull();
  });

  it('a verified flag with an empty basis still says it is verified, and shows no invented reason', () => {
    const outcome = hashOutcome(hash({ verified: true, basis: '' }));
    expect(outcome.tone).toBe('verified');
    expect(outcome.basis).toBe('');
  });
});
