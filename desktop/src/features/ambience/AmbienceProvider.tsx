/**
 * AmbienceProvider — global ambience context.
 *
 * Mounted once in App.tsx above the page switch. Owns: reading/persisting the
 * ambience settings, mounting the canvas (or not, when disabled), reacting to
 * world events (toasts, journal, carry tag), and exposing the engine handle to
 * the High Interaction UI so it can force a profile while it is open.
 *
 * The only outside-in dependency of the ambience layer is this provider
 * reading its own settings.
 */

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { AmbienceCanvas } from './AmbienceCanvas';
import { AmbienceEngine, type AmbienceProfile } from './engine/engine';
import type { AmbienceEvent } from './engine/state';
import type { JournalData } from './engine/eggs';
import { loadAmbienceSettings, saveAmbienceSettings, type AmbienceSettings } from './ambienceSettings';

export interface AmbienceContextValue {
  /** The effective profile the canvas renders with. */
  profile: AmbienceProfile;
  /** True when ambience is on at all (profile !== 'off'). */
  enabled: boolean;
  setEnabled: (on: boolean) => void;
  soundOn: boolean;
  setSoundOn: (on: boolean) => void;
  musicVolume: number;
  setMusicVolume: (v: number) => void;
  /** SFX loudness setting (0..1; 0.5 = the default level, 1 = louder). */
  soundVolume: number;
  setSoundVolume: (v: number) => void;
  /** Effective music on/off: the user's preference AND a loud profile. */
  musicOn: boolean;
  setMusicOn: (on: boolean) => void;
  /** Latest Field Journal snapshot (refreshed on discovery). */
  journal: JournalData | null;
  /** The most recent discovery/achievement event (for toasts). */
  lastEvent: AmbienceEvent | null;
  /** Force a profile while a surface is open (e.g. the Lab bench drops to calm). */
  overrideProfile: (p: AmbienceProfile | null) => void;
  /** Hide the standard page background (0% opacity) behind the world. */
  clearBackground: boolean;
  setClearBackground: (on: boolean) => void;
  /**
   * The living background is off because motion is reduced, not because the
   * user switched it off. Settings uses this to explain the disabled control
   * instead of silently ignoring a click.
   */
  motionSuppressed: boolean;
  /** Living-background page controls (proxied to the engine). */
  setTod: (t: number) => void;
  setWeather: (w: 'clear' | 'rain' | 'snow') => void;
  /** Pin the time of day / weather so the world stops changing them itself. */
  setTodLocked: (on: boolean) => void;
  setWeatherLocked: (on: boolean) => void;
  /** Read the world's clock so controls can follow it. Null before the engine exists. */
  readClock: () => AmbienceClock | null;
  setZoom: (z: number) => void;
  setBuddy: (on: boolean) => void;
  /** Rainbow: put one up now, or take it down. */
  setRainbow: (on: boolean) => void;
  /** Music: pick a piece, an instrument, or hand it back to autoplay. */
  setTrack: (id: string) => void;
  setInstrument: (id: string) => void;
  /**
   * Hand the choice of piece back to autoplay ("Let it choose"). Engine-only,
   * deliberately not persisted: it is implied by the piece selector, which
   * itself resets to "Let it choose" every time the panel mounts, so a stored
   * `false` would silently outlive the pick that caused it.
   */
  setMusicAuto: (on: boolean) => void;
  /**
   * "Let it choose", picked just now: autoplay again, starting on a fresh
   * piece rather than waiting out the one that is pinned.
   */
  shuffleMusic: () => void;
  ready: boolean;
}

/** A snapshot of the world's clock, for controls that mirror it. */
export interface AmbienceClock {
  tod: number;
  weather: 'clear' | 'rain' | 'snow';
  todLocked: boolean;
  weatherLocked: boolean;
}

const AmbienceContext = createContext<AmbienceContextValue | null>(null);

const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)';

/**
 * Is motion reduced right now?
 *
 * The app's own Motion setting is published as `data-motion` on <html>, which
 * is how the ambience layer reads it without importing the theme provider.
 * An explicit `full` WINS over the OS query — otherwise a user on a machine
 * with system-wide reduced motion could never turn the living world back on,
 * which matters now that reduced motion switches it off.
 */
function reducedMotionPref(): boolean {
  const attr = typeof document !== 'undefined'
    ? document.documentElement.getAttribute('data-motion')
    : null;
  if (attr === 'reduced') return true;
  if (attr === 'full') return false;
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia(REDUCED_MOTION_QUERY).matches
    : false;
}

export function AmbienceProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<AmbienceSettings | null>(null);
  const [override, setOverride] = useState<AmbienceProfile | null>(null);
  const [journal, setJournal] = useState<JournalData | null>(null);
  const [lastEvent, setLastEvent] = useState<AmbienceEvent | null>(null);
  const [engine, setEngine] = useState<AmbienceEngine | null>(null);
  /**
   * Reduced motion is LIVE state, not a mount-time snapshot: it now decides
   * whether the living world runs at all, so the switch has to follow the
   * Motion setting (and the OS query) as they change.
   */
  const [reducedMotion, setReducedMotion] = useState(reducedMotionPref);
  const overrideRef = useRef<AmbienceProfile | null>(null);
  overrideRef.current = override;

  const refreshJournal = useCallback(() => {
    setJournal(engine ? engine.journal() : null);
  }, [engine]);

  useEffect(() => { refreshJournal(); }, [refreshJournal]);

  useEffect(() => {
    let alive = true;
    void loadAmbienceSettings().then((s) => { if (alive) setSettings(s); });
    return () => { alive = false; };
  }, []);

  // Track the Motion setting (via `data-motion`) and the OS query together.
  useEffect(() => {
    const sync = () => setReducedMotion(reducedMotionPref());
    sync();
    const observer = typeof MutationObserver !== 'undefined'
      ? new MutationObserver(sync)
      : null;
    observer?.observe(document.documentElement, { attributes: true, attributeFilter: ['data-motion'] });
    const media = typeof window !== 'undefined' && typeof window.matchMedia === 'function'
      ? window.matchMedia(REDUCED_MOTION_QUERY)
      : null;
    media?.addEventListener('change', sync);
    return () => {
      observer?.disconnect();
      media?.removeEventListener('change', sync);
    };
  }, []);

  // expose the ambience state to CSS (translucent shell over the world)
  useEffect(() => {
    const root = document.documentElement;
    const active = settings && settings.enabled && !reducedMotion ? (overrideRef.current ?? 'full') : 'off';
    root.setAttribute('data-ambience', active);
    // Background removal only ever means "let the living world show through",
    // so it must follow the world: with nothing rendering behind it, a 0%
    // background is just a hole.
    root.setAttribute('data-ambience-clear', active !== 'off' && settings?.clearBackground ? 'on' : 'off');
    return () => {
      root.removeAttribute('data-ambience');
      root.removeAttribute('data-ambience-clear');
    };
  }, [settings, override, reducedMotion]);

  // The world runs at `full` whenever it is on. There used to be a calm/full
  // "background intensity" setting, but it never took: the coordinator forces a
  // profile per surface, so the stored value was overwritten before it could be
  // seen. A surface that wants the quiet version still asks for it through
  // `overrideProfile` (the Lab bench does).
  //
  // Reduced motion switches the living world OFF entirely. It is a wandering,
  // weather-changing, animated background whose whole point is movement, so
  // "reduce motion" cannot honestly mean "same world, held still". The stored
  // `enabled` flag is left untouched, so the world comes back by itself when
  // motion is allowed again.
  const effectiveProfile: AmbienceProfile = !settings || !settings.enabled || reducedMotion
    ? 'off'
    : (overrideRef.current ?? 'full');

  const persist = useCallback((patch: Partial<AmbienceSettings>) => {
    setSettings((cur) => {
      if (!cur) return cur;
      const next = { ...cur, ...patch };
      void saveAmbienceSettings(next);
      return next;
    });
  }, []);

  const handleEvent = useCallback((ev: AmbienceEvent) => {
    if (ev.type === 'discovery') {
      setJournal(engine ? engine.journal() : null);
    }
    setLastEvent(ev);
  }, [engine]);

  const value = useMemo<AmbienceContextValue>(() => ({
    profile: effectiveProfile,
    enabled: effectiveProfile !== 'off',
    // Turning the world off takes background removal with it: the setting only
    // exists to reveal the world, and it is the one ambience option that keeps
    // changing the page after the world stops rendering.
    setEnabled: (on) => persist(on ? { enabled: true } : { enabled: false, clearBackground: false }),
    soundOn: settings?.sound ?? false,
    setSoundOn: (on) => persist({ sound: on }),
    musicVolume: settings?.musicVolume ?? 0.35,
    setMusicVolume: (v) => persist({ musicVolume: v }),
    soundVolume: settings?.soundVolume ?? 0.5,
    setSoundVolume: (v) => persist({ soundVolume: v }),
    musicOn: (settings?.musicOn !== false) && effectiveProfile === 'full',
    setMusicOn: (on) => persist({ musicOn: on }),
    clearBackground: effectiveProfile !== 'off' && (settings?.clearBackground ?? false),
    setClearBackground: (on) => persist({ clearBackground: on }),
    motionSuppressed: reducedMotion,
    journal,
    lastEvent,
    overrideProfile: (p) => setOverride(p),
    setTod: (t) => engine?.setTod(t),
    setWeather: (w) => engine?.setWeather(w),
    setTodLocked: (on) => engine?.setTodLocked(on),
    setWeatherLocked: (on) => engine?.setWeatherLocked(on),
    readClock: () => (engine ? (engine.clockState() as AmbienceClock) : null),
    setZoom: (z) => engine?.setZoom(z),
    setBuddy: (on) => engine?.setBuddy(on),
    setRainbow: (on) => engine?.setRainbow(on),
    setTrack: (id) => engine?.setTrack(id),
    setInstrument: (id) => engine?.setInstrument(id),
    setMusicAuto: (on) => engine?.setMusicAuto(on),
    shuffleMusic: () => engine?.shuffleNow(),
    ready: settings !== null,
  }), [effectiveProfile, settings, journal, lastEvent, persist, engine, reducedMotion]);

  // Music rides along with the full profile (optional in calm).
  const musicOn = (settings?.musicOn !== false) && effectiveProfile === 'full';

  return (
    <AmbienceContext.Provider value={value}>
      {settings && effectiveProfile !== 'off' && (
        <AmbienceCanvas
          profile={effectiveProfile}
          soundOn={settings.sound}
          // The stored 0..1 loudness becomes the engine's 0..2 multiplier, so
          // the default (0.5) keeps today's level and the top of the slider
          // plays above it.
          soundVolume={(settings.soundVolume ?? 0.5) * 2}
          musicVolume={settings.musicVolume}
          musicOn={musicOn}
          reducedMotion={reducedMotion}
          onEvent={handleEvent}
          onEngineReady={setEngine}
        />
      )}
      {children}
    </AmbienceContext.Provider>
  );
}

export function useAmbience(): AmbienceContextValue {
  const ctx = useContext(AmbienceContext);
  if (!ctx) throw new Error('useAmbience must be used within AmbienceProvider');
  return ctx;
}
