/**
 * AmbienceCanvas — the whole React surface of the ambience layer.
 *
 * One component, two canvases (background world + fx overlay), one engine.
 * React's job is: create the elements, `engine.start()`, `engine.stop()` on
 * unmount, and forward profile/volume changes. Everything else in
 * `engine/` is prototype code in TypeScript — no hooks, no state, no JSX.
 */

import { useEffect, useRef } from 'react';
import { AmbienceEngine, type AmbienceProfile } from './engine/engine';
import type { AmbienceEvent } from './engine/state';

export interface AmbienceCanvasProps {
  profile: AmbienceProfile;
  soundOn: boolean;
  /** SFX loudness multiplier (default 1 = prototype fixed level, 2 = twice as
   * loud). Optional to keep bare test mounts working. */
  soundVolume?: number;
  musicVolume: number;
  musicOn: boolean;
  reducedMotion: boolean;
  onEvent: (ev: AmbienceEvent) => void;
  onEngineReady?: (engine: AmbienceEngine | null) => void;
}

export function AmbienceCanvas({ profile, soundOn, soundVolume = 1, musicVolume, musicOn, reducedMotion, onEvent, onEngineReady }: AmbienceCanvasProps) {
  const bgRef = useRef<HTMLCanvasElement>(null);
  const fxRef = useRef<HTMLCanvasElement>(null);
  const engineRef = useRef<AmbienceEngine | null>(null);
  const onEventRef = useRef(onEvent);
  const firstMusicRef = useRef(true);
  onEventRef.current = onEvent;

  useEffect(() => {
    if (!bgRef.current || !fxRef.current) return;
    const engine = new AmbienceEngine(bgRef.current, fxRef.current, {
      profile,
      musicVolume,
      musicOn,
      soundVolume,
      reducedMotion,
      onEvent: (ev) => onEventRef.current(ev),
    });
    engineRef.current = engine;
    onEngineReady?.(engine);
    engine.start();
    // A user gesture unlocks audio (browsers block autoplay). The engine's
    // own pointerdown kick also starts music when musicOn; this is the
    // explicit resume for SFX.
    // ONE-SHOT. Browsers block audio until a gesture, so the first pointerdown
    // unlocks it — but this used to stay registered, so every subsequent click
    // anywhere in the app re-entered unlockAudio() and restarted the music with
    // a freshly picked track. Clicking anything changed the song.
    const unlock = () => {
      window.removeEventListener('pointerdown', unlock);
      engine.unlockAudio();
    };
    window.addEventListener('pointerdown', unlock, { once: true });
    return () => {
      window.removeEventListener('pointerdown', unlock);
      engine.stop();
      engineRef.current = null;
      onEngineReady?.(null);
    };
    // Mount once; profile/volume changes go through the effects below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => { engineRef.current?.setProfile(profile); }, [profile]);
  useEffect(() => { engineRef.current?.setSoundOn(soundOn); }, [soundOn]);
  useEffect(() => { engineRef.current?.setSoundVolume(soundVolume); }, [soundVolume]);
  useEffect(() => { engineRef.current?.setMusicVolume(musicVolume); }, [musicVolume]);
  // Music never starts before a user gesture: the engine's own pointerdown
  // kick starts it on first run; later changes are user-driven anyway.
  useEffect(() => {
    if (firstMusicRef.current) { firstMusicRef.current = false; return; }
    engineRef.current?.setMusicOn(musicOn);
  }, [musicOn]);

  return (
    <>
      <canvas ref={bgRef} className="ambience-canvas" aria-hidden="true" />
      <div className="ambience-vig" aria-hidden="true" />
      <canvas ref={fxRef} className="ambience-fx" aria-hidden="true" />
    </>
  );
}
