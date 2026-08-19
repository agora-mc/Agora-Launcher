/**
 * Ambience SFX — `blip` / `chord`.
 *
 * `soundOn` lives in the engine state so the ambience canvas and the High
 * Interaction UI share the same mute flag.
 */

import type { EngineState } from '../state';

let ac: AudioContext | null = null;

function audioContext(): AudioContext | null {
  try {
    const Ctor = window.AudioContext || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    return Ctor ? new Ctor() : null;
  } catch {
    return null;
  }
}

export function blip(state: EngineState, f: number, d?: number, t?: OscillatorType, v?: number): void {
  if (!state.soundOn) return;
  try {
    ac = ac || audioContext();
    if (!ac) return;
    const o = ac.createOscillator(), g = ac.createGain();
    o.type = t || 'triangle'; o.frequency.value = f;
    // `state.soundVolume` is a loudness multiplier on the note's own gain, so
    // the volume slider can push SFX above the prototype's fixed level.
    const peak = Math.min(0.9, Math.max(0.0001, (v || 0.08) * (state.soundVolume ?? 1)));
    g.gain.setValueAtTime(0.0001, ac.currentTime);
    g.gain.exponentialRampToValueAtTime(peak, ac.currentTime + 0.012);
    g.gain.exponentialRampToValueAtTime(0.0001, ac.currentTime + (d || 0.12));
    o.connect(g); g.connect(ac.destination); o.start(); o.stop(ac.currentTime + (d || 0.12) + 0.03);
  } catch {
    // never let SFX break the world
  }
}

export function chord(state: EngineState, list: number[], gap?: number): void {
  list.forEach((f, i) => { setTimeout(() => blip(state, f, 0.16), i * (gap || 70)); });
}

/** Resume the shared context after a user gesture (browsers block autoplay). */
export function resumeSfx(): void {
  try {
    if (ac && ac.state === 'suspended') void ac.resume();
  } catch {
    // ignore
  }
}
