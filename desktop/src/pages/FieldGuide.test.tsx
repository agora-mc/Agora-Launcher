/**
 * The Field Guide's intro line must describe what the living background is
 * doing right now. With Reduce motion on, the world is held as one still
 * picture, so the page must not say it is running.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

const ambience = { enabled: true, reducedMotion: false, journal: null as null | object };
vi.mock('../features/ambience/AmbienceProvider', () => ({ useAmbience: () => ambience }));
vi.mock('../features/ambience/journalStorage', () => ({
  getCachedJournalRawSync: () => null,
  loadJournalRaw: async () => null,
}));
vi.mock('../features/ambience/FieldJournal', () => ({ FieldJournalView: () => null }));

import { FieldGuide } from './FieldGuide';

describe('FieldGuide background status line', () => {
  beforeEach(() => {
    ambience.enabled = true;
    ambience.reducedMotion = false;
  });

  it('says the background is running when it is on and moving', () => {
    render(<FieldGuide />);
    expect(screen.getByText(/The background is running/)).toBeTruthy();
  });

  it('says the background holds still when Reduce motion is on, and never claims it is running', () => {
    ambience.reducedMotion = true;
    render(<FieldGuide />);
    expect(screen.getByText(/holds still as one picture/)).toBeTruthy();
    expect(screen.queryByText(/The background is running/)).toBeNull();
  });

  it('asks the player to turn the background on when it is off', () => {
    ambience.enabled = false;
    render(<FieldGuide />);
    expect(screen.getByText(/Turn the living background on to find more/)).toBeTruthy();
  });
});
