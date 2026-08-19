/**
 * Ambience settings — the only place the ambience layer touches persisted
 * settings. Written through the app's `getSetting`/`setSetting` (tauri) with
 * a localStorage fallback so the ambience still works in plain-browser dev
 * and tests. This is presentation state; no instance data is stored here.
 */

import { getSetting, setSetting } from '@/lib/tauri';

export const AMBIENCE_ENABLED_KEY = 'ambience.enabled';
export const AMBIENCE_MUSIC_VOLUME_KEY = 'ambience.music-volume';
export const AMBIENCE_SOUND_KEY = 'ambience.sound';
export const AMBIENCE_CLEAR_BACKGROUND_KEY = 'ambience.clear-background';

const DEFAULTS = {
  enabled: true,
  musicVolume: 0.35,
  sound: false,
  clearBackground: false,
};

async function read<T>(key: string, fallback: T): Promise<T> {
  try {
    const v = await getSetting(key);
    if (v !== null && v !== undefined) return v as T;
  } catch {
    // fall through to localStorage
  }
  try {
    const raw = window.localStorage.getItem(key);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

async function write(key: string, value: unknown): Promise<void> {
  try {
    await setSetting(key, value);
    return;
  } catch {
    // fall through to localStorage
  }
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // best effort
  }
}

export interface AmbienceSettings {
  enabled: boolean;
  musicVolume: number;
  sound: boolean;
  /** Hide the standard page background (0% opacity) so the living world shows
   * through crystal clear instead of behind a translucent shell. */
  clearBackground: boolean;
}

export async function loadAmbienceSettings(): Promise<AmbienceSettings> {
  const [enabled, musicVolume, sound, clearBackground] = await Promise.all([
    read<boolean>(AMBIENCE_ENABLED_KEY, DEFAULTS.enabled),
    read<number>(AMBIENCE_MUSIC_VOLUME_KEY, DEFAULTS.musicVolume),
    read<boolean>(AMBIENCE_SOUND_KEY, DEFAULTS.sound),
    read<boolean>(AMBIENCE_CLEAR_BACKGROUND_KEY, DEFAULTS.clearBackground),
  ]);
  return {
    enabled: enabled === true,
    musicVolume: Math.max(0, Math.min(1, typeof musicVolume === 'number' ? musicVolume : DEFAULTS.musicVolume)),
    sound: sound === true,
    clearBackground: clearBackground === true,
  };
}

export async function saveAmbienceSettings(s: AmbienceSettings): Promise<void> {
  await Promise.all([
    write(AMBIENCE_ENABLED_KEY, s.enabled),
    write(AMBIENCE_MUSIC_VOLUME_KEY, s.musicVolume),
    write(AMBIENCE_SOUND_KEY, s.sound),
    write(AMBIENCE_CLEAR_BACKGROUND_KEY, s.clearBackground),
  ]);
}
