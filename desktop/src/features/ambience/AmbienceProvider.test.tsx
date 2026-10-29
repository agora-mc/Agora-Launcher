/**
 * The living background under reduced motion. The world is held still, not
 * switched off: the canvas stays mounted, the context still reports the world
 * as on (so the sidebar entry stays), and the page keeps its translucent shell.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import type { AmbienceSettings } from './ambienceSettings';

const canvasProps: Array<{ reducedMotion: boolean; profile: string }> = [];

vi.mock('./AmbienceCanvas', () => ({
  AmbienceCanvas: (props: { reducedMotion: boolean; profile: string }) => {
    canvasProps.push({ reducedMotion: props.reducedMotion, profile: props.profile });
    return null;
  },
}));

const stored: AmbienceSettings = {
  enabled: true,
  musicVolume: 0.35,
  sound: false,
  soundVolume: 0.5,
  musicOn: true,
  clearBackground: false,
};

vi.mock('./ambienceSettings', () => ({
  loadAmbienceSettings: async () => stored,
  saveAmbienceSettings: async () => undefined,
}));

vi.mock('./journalStorage', () => ({
  ensureJournalLoaded: async () => null,
  getCachedJournalRawSync: () => null,
  saveJournalRaw: async () => undefined,
}));

import { AmbienceProvider, useAmbience } from './AmbienceProvider';

function Probe() {
  const ambience = useAmbience();
  return (
    <div>
      <span data-testid="enabled">{String(ambience.enabled)}</span>
      <span data-testid="profile">{ambience.profile}</span>
      <span data-testid="ready">{String(ambience.ready)}</span>
    </div>
  );
}

function mountProvider() {
  return render(
    <AmbienceProvider>
      <Probe />
    </AmbienceProvider>,
  );
}

beforeEach(() => {
  canvasProps.length = 0;
  stored.enabled = true;
  document.documentElement.removeAttribute('data-motion');
  document.documentElement.removeAttribute('data-ambience');
});

afterEach(() => {
  document.documentElement.removeAttribute('data-motion');
  document.documentElement.removeAttribute('data-ambience');
  vi.restoreAllMocks();
});

describe('AmbienceProvider under reduced motion', () => {
  it('keeps the world on and mounts the canvas still, with the Motion setting reduced', async () => {
    document.documentElement.setAttribute('data-motion', 'reduced');
    mountProvider();

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('true'));
    expect(screen.getByTestId('enabled').textContent).toBe('true');
    expect(screen.getByTestId('profile').textContent).toBe('full');
    const last = canvasProps[canvasProps.length - 1];
    expect(last).toEqual({ reducedMotion: true, profile: 'full' });
    expect(document.documentElement.getAttribute('data-ambience')).toBe('full');
  });

  it('still turns the world off when the user has switched it off', async () => {
    stored.enabled = false;
    document.documentElement.setAttribute('data-motion', 'reduced');
    mountProvider();

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('true'));
    expect(screen.getByTestId('enabled').textContent).toBe('false');
    expect(screen.getByTestId('profile').textContent).toBe('off');
    expect(canvasProps).toHaveLength(0);
  });

  it('with full motion the canvas gets reducedMotion false', async () => {
    document.documentElement.setAttribute('data-motion', 'full');
    mountProvider();

    await waitFor(() => expect(screen.getByTestId('ready').textContent).toBe('true'));
    expect(canvasProps[canvasProps.length - 1]).toEqual({ reducedMotion: false, profile: 'full' });
  });
});
