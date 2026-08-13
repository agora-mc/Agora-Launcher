/**
 * Ambience settings — the only place the ambience layer touches persisted
 * settings. Written through the app's `getSetting`/`setSetting` (tauri) with
 * a localStorage fallback so the ambience still works in plain-browser dev
 * and tests. This is presentation state; no instance data is stored here.
 */

import { getSetting, setSetting } from '@/lib/tauri';
import type { AmbienceProfile } from './engine/engine';

export const AMBIENCE_ENABLED_KEY = 'ambience.enabled';
export const AMBIENCE_PROFILE_KEY = 'ambience.profile';
export const AMBIENCE_MUSIC_VOLUME_KEY = 'ambience.music-volume';
export const AMBIENCE_SOUND_KEY = 'ambience.sound';

const DEFAULTS = {
  enabled: true,
  profile: 'calm' as AmbienceProfile,
  musicVolume: 0.35,
  sound: false,
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
  profile: AmbienceProfile;
  musicVolume: number;
  sound: boolean;
}

export async function loadAmbienceSettings(): Promise<AmbienceSettings> {
  const [enabled, profile, musicVolume, sound] = await Promise.all([
    read<boolean>(AMBIENCE_ENABLED_KEY, DEFAULTS.enabled),
    read<string>(AMBIENCE_PROFILE_KEY, DEFAULTS.profile),
    read<number>(AMBIENCE_MUSIC_VOLUME_KEY, DEFAULTS.musicVolume),
    read<boolean>(AMBIENCE_SOUND_KEY, DEFAULTS.sound),
  ]);
  const p: AmbienceProfile = profile === 'full' || profile === 'calm' ? profile : 'calm';
  return {
    enabled: enabled === true,
    profile: p,
    musicVolume: Math.max(0, Math.min(1, typeof musicVolume === 'number' ? musicVolume : DEFAULTS.musicVolume)),
    sound: sound === true,
  };
}

export async function saveAmbienceSettings(s: AmbienceSettings): Promise<void> {
  await Promise.all([
    write(AMBIENCE_ENABLED_KEY, s.enabled),
    write(AMBIENCE_PROFILE_KEY, s.profile),
    write(AMBIENCE_MUSIC_VOLUME_KEY, s.musicVolume),
    write(AMBIENCE_SOUND_KEY, s.sound),
  ]);
}
