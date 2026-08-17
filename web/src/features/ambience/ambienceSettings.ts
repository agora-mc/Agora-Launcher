/**
 * Ambience settings — the only place the ambience layer touches persisted
 * settings. Web copy: the desktop original writes through the app's
 * `getSetting`/`setSetting` (tauri) with a localStorage fallback; the site has
 * no tauri, so the localStorage path is the only path. This is presentation
 * state; no instance data is stored here. (Not currently consumed by the web
 * diorama — kept so the feature copy stays self-contained and extractable.)
 */

export const AMBIENCE_ENABLED_KEY = 'ambience.enabled';
export const AMBIENCE_MUSIC_VOLUME_KEY = 'ambience.music-volume';
export const AMBIENCE_SOUND_KEY = 'ambience.sound';
export const AMBIENCE_CLEAR_BACKGROUND_KEY = 'ambience.clear-background';
export const AMBIENCE_MUSIC_AUTO_KEY = 'ambience.music-auto';

const DEFAULTS = {
  enabled: true,
  musicVolume: 0.35,
  sound: false,
  clearBackground: false,
  musicAuto: true,
};

async function read<T>(key: string, fallback: T): Promise<T> {
  try {
    const raw = window.localStorage.getItem(key);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

async function write(key: string, value: unknown): Promise<void> {
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
  /** Autoplay rotates pieces when one ends; off pins the chosen track. */
  musicAuto: boolean;
}

export async function loadAmbienceSettings(): Promise<AmbienceSettings> {
  const [enabled, musicVolume, sound, clearBackground, musicAuto] = await Promise.all([
    read<boolean>(AMBIENCE_ENABLED_KEY, DEFAULTS.enabled),
    read<number>(AMBIENCE_MUSIC_VOLUME_KEY, DEFAULTS.musicVolume),
    read<boolean>(AMBIENCE_SOUND_KEY, DEFAULTS.sound),
    read<boolean>(AMBIENCE_CLEAR_BACKGROUND_KEY, DEFAULTS.clearBackground),
    read<boolean>(AMBIENCE_MUSIC_AUTO_KEY, DEFAULTS.musicAuto),
  ]);
  return {
    enabled: enabled === true,
    musicVolume: Math.max(0, Math.min(1, typeof musicVolume === 'number' ? musicVolume : DEFAULTS.musicVolume)),
    sound: sound === true,
    clearBackground: clearBackground === true,
    musicAuto: musicAuto !== false,
  };
}

export async function saveAmbienceSettings(s: AmbienceSettings): Promise<void> {
  await Promise.all([
    write(AMBIENCE_ENABLED_KEY, s.enabled),
    write(AMBIENCE_MUSIC_VOLUME_KEY, s.musicVolume),
    write(AMBIENCE_SOUND_KEY, s.sound),
    write(AMBIENCE_CLEAR_BACKGROUND_KEY, s.clearBackground),
    write(AMBIENCE_MUSIC_AUTO_KEY, s.musicAuto),
  ]);
}
