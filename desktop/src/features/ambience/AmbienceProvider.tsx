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
  /** Latest Field Journal snapshot (refreshed on discovery). */
  journal: JournalData | null;
  /** The most recent discovery/achievement event (for toasts). */
  lastEvent: AmbienceEvent | null;
  /** Force a profile while a surface is open (e.g. the Lab bench drops to calm). */
  overrideProfile: (p: AmbienceProfile | null) => void;
  /** Hide the standard page background (0% opacity) behind the world. */
  clearBackground: boolean;
  setClearBackground: (on: boolean) => void;
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

function reducedMotionPref(): boolean {
  if (typeof document !== 'undefined') {
    const attr = document.documentElement.getAttribute('data-motion');
    if (attr === 'reduced') return true;
  }
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia('(prefers-reduced-motion: reduce)').matches
    : false;
}

export function AmbienceProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<AmbienceSettings | null>(null);
  const [override, setOverride] = useState<AmbienceProfile | null>(null);
  const [journal, setJournal] = useState<JournalData | null>(null);
  const [lastEvent, setLastEvent] = useState<AmbienceEvent | null>(null);
  const [engine, setEngine] = useState<AmbienceEngine | null>(null);
  const reducedMotion = useMemo(reducedMotionPref, []);
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

  // expose the ambience state to CSS (translucent shell over the world)
  useEffect(() => {
    const root = document.documentElement;
    const active = settings && settings.enabled ? (overrideRef.current ?? 'full') : 'off';
    root.setAttribute('data-ambience', active);
    root.setAttribute('data-ambience-clear', settings?.clearBackground ? 'on' : 'off');
    return () => {
      root.removeAttribute('data-ambience');
      root.removeAttribute('data-ambience-clear');
    };
  }, [settings, override]);

  // The world runs at `full` whenever it is on. There used to be a calm/full
  // "background intensity" setting, but it never took: the coordinator forces a
  // profile per surface, so the stored value was overwritten before it could be
  // seen. A surface that wants the quiet version still asks for it through
  // `overrideProfile` (the Lab bench does).
  const effectiveProfile: AmbienceProfile = !settings || !settings.enabled
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
    setEnabled: (on) => persist({ enabled: on }),
    soundOn: settings?.sound ?? false,
    setSoundOn: (on) => persist({ sound: on }),
    musicVolume: settings?.musicVolume ?? 0.35,
    setMusicVolume: (v) => persist({ musicVolume: v }),
    clearBackground: settings?.clearBackground ?? false,
    setClearBackground: (on) => persist({ clearBackground: on }),
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
    ready: settings !== null,
  }), [effectiveProfile, settings, journal, lastEvent, persist, engine]);

  // Music rides along with the full profile (optional in calm).
  const musicOn = effectiveProfile === 'full';

  return (
    <AmbienceContext.Provider value={value}>
      {settings && effectiveProfile !== 'off' && (
        <AmbienceCanvas
          profile={effectiveProfile}
          soundOn={settings.sound}
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
