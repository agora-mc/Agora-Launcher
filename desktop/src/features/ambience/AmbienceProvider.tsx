/**
 * AmbienceProvider — global ambience context.
 *
 * Mounted once in App.tsx above the page switch. Owns: reading/persisting the
 * ambience settings, mounting the canvas (or not, when disabled), reacting to
 * world events (toasts, journal, carry tag), and exposing the engine handle to
 * the High Interaction UI so it can force a profile while it is open.
 *
 * The only outside-in dependency of the ambience layer is this provider
 * reading its own settings (V5-PORT-PLAN §3).
 */

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { AmbienceCanvas } from './AmbienceCanvas';
import { AmbienceEngine, type AmbienceProfile } from './engine/engine';
import type { AmbienceEvent } from './engine/state';
import type { JournalData } from './engine/eggs';
import { loadAmbienceSettings, saveAmbienceSettings } from './ambienceSettings';

export interface AmbienceContextValue {
  /** The effective profile the canvas renders with. */
  profile: AmbienceProfile;
  /** True when ambience is on at all (profile !== 'off'). */
  enabled: boolean;
  setEnabled: (on: boolean) => void;
  setProfile: (p: AmbienceProfile) => void;
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
  ready: boolean;
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
  const [settings, setSettings] = useState<{ enabled: boolean; profile: AmbienceProfile; musicVolume: number; sound: boolean } | null>(null);
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
    const active = settings && settings.enabled ? (overrideRef.current ?? settings.profile) : 'off';
    root.setAttribute('data-ambience', active);
    return () => { root.removeAttribute('data-ambience'); };
  }, [settings, override]);

  const effectiveProfile: AmbienceProfile = !settings || !settings.enabled
    ? 'off'
    : (overrideRef.current ?? settings.profile);

  const persist = useCallback((patch: Partial<{ enabled: boolean; profile: AmbienceProfile; musicVolume: number; sound: boolean }>) => {
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
    setProfile: (p) => persist({ profile: p }),
    soundOn: settings?.sound ?? false,
    setSoundOn: (on) => persist({ sound: on }),
    musicVolume: settings?.musicVolume ?? 0.35,
    setMusicVolume: (v) => persist({ musicVolume: v }),
    journal,
    lastEvent,
    overrideProfile: (p) => setOverride(p),
    ready: settings !== null,
  }), [effectiveProfile, settings, journal, lastEvent, persist]);

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
