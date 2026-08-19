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
export const AMBIENCE_SOUND_VOLUME_KEY = 'ambience.sound-volume';
export const AMBIENCE_MUSIC_ON_KEY = 'ambience.music-on';
export const AMBIENCE_CLEAR_BACKGROUND_KEY = 'ambience.clear-background';

const DEFAULTS = {
  enabled: true,
  musicVolume: 0.35,
  sound: false,
  // 0..1 stored loudness. Falls halfway up the slider (50) so it can be turned
  // up past today's fixed SFX level: at 0.5 the engine's 1× multiplier keeps
  // the current loudness, at 1 it plays twice as loud.
  soundVolume: 0.5,
  musicOn: true,
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
  /** Volumes for the living world's SFX (0..1). The slider maps to a 0..2
   * engine multiplier so the default (0.5) matches today's loudness and the
   * top of the range plays above it. */
  soundVolume: number;
  /** Whether the music engine is allowed to play at all. Independent of the
   * volume slider and of autoplay: this is the master music on/off. */
  musicOn: boolean;
  /** Hide the standard page background (0% opacity) so the living world shows
   * through crystal clear instead of behind a translucent shell. */
  clearBackground: boolean;
}

export async function loadAmbienceSettings(): Promise<AmbienceSettings> {
<<<<<<< HEAD
  const [enabled, musicVolume, sound, soundVolume, musicOn, clearBackground, musicAuto] = await Promise.all([
=======
  const [enabled, musicVolume, sound, clearBackground] = await Promise.all([
>>>>>>> claude/living-background-autoplay-songs-8180f6
    read<boolean>(AMBIENCE_ENABLED_KEY, DEFAULTS.enabled),
    read<number>(AMBIENCE_MUSIC_VOLUME_KEY, DEFAULTS.musicVolume),
    read<boolean>(AMBIENCE_SOUND_KEY, DEFAULTS.sound),
    read<number>(AMBIENCE_SOUND_VOLUME_KEY, DEFAULTS.soundVolume),
    read<boolean>(AMBIENCE_MUSIC_ON_KEY, DEFAULTS.musicOn),
    read<boolean>(AMBIENCE_CLEAR_BACKGROUND_KEY, DEFAULTS.clearBackground),
  ]);
  return {
    enabled: enabled === true,
    musicVolume: Math.max(0, Math.min(1, typeof musicVolume === 'number' ? musicVolume : DEFAULTS.musicVolume)),
    sound: sound === true,
    soundVolume: Math.max(0, Math.min(1, typeof soundVolume === 'number' ? soundVolume : DEFAULTS.soundVolume)),
    musicOn: musicOn !== false,
    clearBackground: clearBackground === true,
  };
}

export async function saveAmbienceSettings(s: AmbienceSettings): Promise<void> {
  await Promise.all([
    write(AMBIENCE_ENABLED_KEY, s.enabled),
    write(AMBIENCE_MUSIC_VOLUME_KEY, s.musicVolume),
    write(AMBIENCE_SOUND_KEY, s.sound),
    write(AMBIENCE_SOUND_VOLUME_KEY, s.soundVolume),
    write(AMBIENCE_MUSIC_ON_KEY, s.musicOn),
    write(AMBIENCE_CLEAR_BACKGROUND_KEY, s.clearBackground),
  ]);
}
