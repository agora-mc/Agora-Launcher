/**
 * Ambience music engine — ported from `docs/interactive/prototypes/music-preview.html`
 * (the A1 reference engine). The v4-world sketch used a 12th-note grid that
 * cannot represent the data: Moonlight's arpeggios are ⅓ of a beat, Clair de
 * Lune has 0.75 and 4.5. Each voice keeps its own cursor and walks its
 * `[pitch, beats]` list — a fixed grid silently drops every event between
 * slots. (V5-PORT-PLAN trap 6.)
 *
 * Also kept from the prototype, both verbatim in intent:
 *  - Register rolloff + highshelf: equal gain is not equal loudness. Sugar
 *    Plum reaches B7 with 27% of its notes above C6; without the rolloff it
 *    is genuinely painful. (trap 7)
 *  - Nothing plays before a user gesture; the AudioContext is created and
 *    resumed inside the first play() call.
 *  - `document.hidden` suspends the context (same rule as the world loop).
 */

import type { MusicTrack, MusicVoice } from './tracks';

export type { MusicTrack };

const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
const LOOKAHEAD = 0.15;
const TICK = 25;

export interface MusicState {
  track: MusicTrack;
  loops: number;
  t0: number;
  beat: number;
  voices: Array<{ v: MusicVoice; i: number; time: number; done: boolean }>;
}

/** Frequency of a note name: "C#4" | "Bb3" | "R". A4 = 440, middle C = C4 = 261.6. */
export function noteFreq(name: string, transpose = 0): number {
  if (name === 'R') return 0;
  const m = /^([A-G]#?)(-?\d+)$/.exec(name);
  if (!m) return 0;
  const midi = (+m[2] + 1) * 12 + NAMES.indexOf(m[1]) + transpose;
  return 440 * Math.pow(2, (midi - 69) / 12);
}

/**
 * Register compensation. Equal gain across the range is not equal loudness:
 * the ear peaks around 2-5 kHz, so a note at C7 reads as piercing at a level
 * that is comfortable at C4. Real instruments roll off up there; this does
 * the same, with a floor so highs stay present.
 */
export function rolloff(f: number, tame = true): number {
  if (!tame || f <= 600) return 1;
  return Math.max(0.3, Math.pow(600 / f, 0.7));
}

const EPS = 0.0002;

function env(p: AudioParam, at: number, dur: number, peak: number, atk: number, rel: number): void {
  atk = Math.min(atk, dur * 0.3); rel = Math.min(rel, dur * 0.6);
  peak = Math.max(peak, EPS);
  p.setValueAtTime(EPS, at);
  p.exponentialRampToValueAtTime(peak, at + atk);
  p.setValueAtTime(peak, Math.max(at + atk, at + dur - rel));
  p.exponentialRampToValueAtTime(EPS, at + dur);
}

function decay(p: AudioParam, at: number, peak: number, tail: number): void {
  peak = Math.max(peak, EPS);
  p.setValueAtTime(EPS, at);
  p.exponentialRampToValueAtTime(peak, at + 0.006);
  p.exponentialRampToValueAtTime(EPS, at + tail);
}

function ring(dur: number, mult: number, cap: number): number {
  return Math.min(cap, Math.max(0.28, dur * mult));
}

/* Karplus-Strong: a burst of noise in a delay line the length of one period,
   fed back through an averaging lowpass. Buffers are cached; they are cheap
   but Bumblebee asks for ~13 notes a second. */
const ksCache: Record<string, AudioBuffer> = {};
const ksKeys: string[] = [];

function ksBuf(ctx: AudioContext, f: number, tail: number): AudioBuffer {
  const key = f.toFixed(1) + '|' + tail.toFixed(2);
  if (ksCache[key]) return ksCache[key];
  const sr = ctx.sampleRate, n = Math.max(2, Math.round(sr / f));
  const len = Math.max(2, Math.ceil(sr * tail));
  const b = ctx.createBuffer(1, len, sr), d = b.getChannelData(0);
  const r = new Float32Array(n);
  for (let i = 0; i < n; i++) r[i] = Math.random() * 2 - 1;
  const dec = Math.pow(0.0006, 1 / (tail * sr));
  let idx = 0;
  for (let i = 0; i < len; i++) {
    d[i] = r[idx];
    const nx = r[(idx + 1) % n];
    r[idx] = 0.5 * (r[idx] + nx) * dec;
    idx = (idx + 1) % n;
  }
  if (ksKeys.length > 500) delete ksCache[ksKeys.shift()!];
  ksKeys.push(key); ksCache[key] = b;
  return b;
}

let organWave: PeriodicWave | null = null;
function organW(ctx: AudioContext): PeriodicWave {
  if (!organWave) {
    // drawbar-ish partials
    const im = new Float32Array([0, 1, 0.5, 0.35, 0.25, 0, 0.16, 0, 0.11]);
    organWave = ctx.createPeriodicWave(new Float32Array(im.length), im);
  }
  return organWave;
}

function osc(ctx: AudioContext, type: OscillatorType | 'organ', f: number, at: number, until: number): OscillatorNode {
  const o = ctx.createOscillator();
  if (type === 'organ') o.setPeriodicWave(organW(ctx)); else o.type = type;
  o.frequency.setValueAtTime(f, at);
  o.start(at); o.stop(until);
  return o;
}

export interface Instrument {
  name: string;
  play: (ctx: AudioContext, master: AudioNode, f: number, at: number, dur: number, g: number) => void;
}

/* Seven synthesis techniques — different methods, not different oscillator
   shapes, so they do not all fail in the same way at the extremes. */
export const INSTRUMENTS: Record<string, Instrument> = {
  chip: {
    name: 'Chiptune',
    play(ctx, master, f, at, dur, g) { // subtractive, square
      const lp = ctx.createBiquadFilter(), gn = ctx.createGain();
      lp.type = 'lowpass'; lp.Q.value = 0.4;
      lp.frequency.setValueAtTime(Math.min(9000, f * 4 + 700), at);
      osc(ctx, 'square', f, at, at + dur + 0.1).connect(lp);
      env(gn.gain, at, dur, g * 0.6, 0.006, 0.1);
      lp.connect(gn); gn.connect(master);
    },
  },
  musicbox: {
    name: 'Music box',
    play(ctx, master, f, at, dur, g) { // additive + fast decay
      const t = ring(dur, 1.6, 2.4);
      [[1, 1, 1], [2, 0.3, 0.7], [3.9, 0.1, 0.45]].forEach((p) => {
        const gn = ctx.createGain();
        osc(ctx, 'sine', f * p[0], at, at + t * p[2] + 0.05).connect(gn);
        decay(gn.gain, at, g * p[1] * 0.9, t * p[2]);
        gn.connect(master);
      });
    },
  },
  rhodes: {
    name: 'Electric piano',
    play(ctx, master, f, at, dur, g) { // FM, 2:1
      const t = ring(dur, 1.5, 2.6), gn = ctx.createGain(), mg = ctx.createGain();
      const car = osc(ctx, 'sine', f, at, at + t + 0.05);
      osc(ctx, 'sine', f * 2, at, at + t + 0.05).connect(mg);
      mg.gain.setValueAtTime(f * 2.2, at);
      mg.gain.exponentialRampToValueAtTime(f * 0.02, at + Math.min(0.5, t));
      mg.connect(car.frequency);
      decay(gn.gain, at, g, t);
      car.connect(gn); gn.connect(master);
    },
  },
  pluck: {
    name: 'Plucked string',
    play(ctx, master, f, at, dur, g) { // Karplus-Strong
      const t = ring(dur, 1.4, 2.2), s = ctx.createBufferSource(), gn = ctx.createGain();
      s.buffer = ksBuf(ctx, f, t);
      gn.gain.setValueAtTime(Math.max(g * 1.6, EPS), at);
      gn.gain.setValueAtTime(Math.max(g * 1.6, EPS), at + t * 0.9);
      gn.gain.exponentialRampToValueAtTime(EPS, at + t);
      s.connect(gn); gn.connect(master);
      s.start(at); s.stop(at + t + 0.02);
    },
  },
  strings: {
    name: 'Strings',
    play(ctx, master, f, at, dur, g) { // detuned saws, slow attack
      const lp = ctx.createBiquadFilter(), gn = ctx.createGain(), until = at + dur + 0.3;
      lp.type = 'lowpass'; lp.Q.value = 0.7;
      lp.frequency.setValueAtTime(Math.min(6500, f * 3.2 + 400), at);
      osc(ctx, 'sawtooth', f * 0.9965, at, until).connect(lp);
      osc(ctx, 'sawtooth', f * 1.0035, at, until).connect(lp);
      env(gn.gain, at, dur, g * 0.5, 0.13, 0.2);
      lp.connect(gn); gn.connect(master);
    },
  },
  organ: {
    name: 'Organ',
    play(ctx, master, f, at, dur, g) { // additive / wavetable
      const gn = ctx.createGain();
      osc(ctx, 'organ', f, at, at + dur + 0.12).connect(gn);
      env(gn.gain, at, dur, g * 0.45, 0.02, 0.09);
      gn.connect(master);
    },
  },
  bell: {
    name: 'Bells',
    play(ctx, master, f, at, dur, g) { // inharmonic FM
      const t = ring(dur, 1.8, 3), gn = ctx.createGain(), mg = ctx.createGain();
      const car = osc(ctx, 'sine', f, at, at + t + 0.05);
      osc(ctx, 'sine', f * 1.41, at, at + t + 0.05).connect(mg);
      mg.gain.setValueAtTime(f * 1.5, at);
      mg.gain.exponentialRampToValueAtTime(f * 0.02, at + t * 0.5);
      mg.connect(car.frequency);
      decay(gn.gain, at, g * 0.85, t);
      car.connect(gn); gn.connect(master);
    },
  },
};

export class MusicEngine {
  private ctx: AudioContext | null = null;
  private master: GainNode | null = null;
  private shelf: BiquadFilterNode | null = null;
  private timer: number | null = null;
  private st: MusicState | null = null;
  private muted: Record<string, boolean> = {};
  private vol = 0.35;
  private loop = true;
  private tame = true;
  private instrument: string | null = null;
  private userPicked = false;
  /** When set, music pauses while hidden (same rule as the world loop). */
  private suspendedForHidden = false;

  setVolume(v: number): void {
    this.vol = Math.max(0, Math.min(1, v));
    if (this.master && this.ctx) {
      const now = this.ctx.currentTime;
      this.master.gain.cancelScheduledValues(now);
      this.master.gain.setValueAtTime(this.vol, now);
    }
  }

  getVolume(): number { return this.vol; }

  setInstrument(id: string | null, byUser = false): void {
    this.instrument = id;
    if (byUser) this.userPicked = true;
  }

  isPlaying(): boolean { return this.st !== null; }

  currentTrackId(): string | null { return this.st ? this.st.track.id : null; }

  /** Duck to ~35% for 450ms so fanfares don't collide, then restore. */
  duck(): void {
    if (!this.master || !this.ctx) return;
    const now = this.ctx.currentTime;
    this.master.gain.cancelScheduledValues(now);
    this.master.gain.setValueAtTime(this.master.gain.value, now);
    this.master.gain.linearRampToValueAtTime(this.vol * 0.35, now + 0.05);
    this.master.gain.linearRampToValueAtTime(this.vol, now + 0.45);
  }

  /** A single note through the current instrument. Used by world SFX that
   * want the music-box colour (e.g. fanfares) without owning a track. */
  tone(freq: number, at: number, dur: number, gain: number): void {
    if (!this.ctx || !this.master) return;
    const peak = gain * this.vol * rolloff(freq, this.tame);
    if (peak <= 0) return;
    (INSTRUMENTS[this.instrument ?? 'chip'] ?? INSTRUMENTS.chip).play(this.ctx, this.master, freq, at, dur, peak);
  }

  private blip(f: number, at: number, dur: number, voice: MusicVoice): void {
    if (!this.ctx || !this.master) return;
    const peak = voice.gain * this.vol * rolloff(f, this.tame);
    if (this.muted[voice.name] || peak <= 0) return;
    (INSTRUMENTS[this.instrument ?? 'chip'] ?? INSTRUMENTS.chip).play(this.ctx, this.master, f, at, dur, peak);
  }

  private ensureGraph(): AudioContext | null {
    if (this.ctx && this.master && this.shelf) return this.ctx;
    try {
      const Ctor = window.AudioContext || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!Ctor) return null;
      this.ctx = new Ctor();
      this.master = this.ctx.createGain(); this.master.gain.value = 1;
      // Shelf, not a lowpass: it pulls down the 2.5 kHz+ band the ear is most
      // sensitive to without dulling everything below it.
      this.shelf = this.ctx.createBiquadFilter();
      this.shelf.type = 'highshelf'; this.shelf.frequency.value = 2600; this.shelf.gain.value = -7;
      this.master.connect(this.shelf); this.shelf.connect(this.ctx.destination);
      return this.ctx;
    } catch {
      return null;
    }
  }

  start(track: MusicTrack, instrument?: string): boolean {
    const ctx = this.ensureGraph();
    if (!ctx || !this.master || !this.shelf) return false;
    if (ctx.state === 'suspended') void ctx.resume();
    this.shelf.gain.value = this.tame ? -7 : 0;
    if (!this.userPicked) this.instrument = instrument ?? track.instrument ?? 'chip';
    this.muted = {};
    const t0 = ctx.currentTime + 0.08;
    this.stop();
    this.st = {
      track,
      loops: 0,
      t0,
      beat: 60 / track.bpm,
      voices: track.voices.map((v) => ({ v, i: 0, time: t0, done: false })),
    };
    this.pump();
    this.timer = window.setInterval(() => this.pump(), TICK);
    return true;
  }

  stop(): void {
    if (this.timer !== null) { clearInterval(this.timer); this.timer = null; }
    this.st = null;
  }

  toggleVoice(name: string): boolean {
    this.muted[name] = !this.muted[name];
    return this.muted[name];
  }

  private pump(): void {
    const st = this.st;
    if (!st || !this.ctx) return;
    const now = this.ctx.currentTime;
    let alive = false;
    st.voices.forEach((vs) => {
      if (vs.done) return;
      alive = true;
      while (vs.time < now + LOOKAHEAD) {
        const ev = vs.v.seq[vs.i], dur = ev[1] * st.beat;
        if (ev[0] !== 'R') {
          const ns = Array.isArray(ev[0]) ? ev[0] : [ev[0]];
          for (const n of ns) this.blip(noteFreq(n), vs.time, dur, vs.v);
        }
        vs.time += dur;
        if (++vs.i >= vs.v.seq.length) {
          vs.i = 0;
          if (!this.loop) { vs.done = true; break; }
          st.loops++;
        }
      }
    });
    if (!alive) this.stop();
  }

  /** Pause the AudioContext while the document is hidden; resume on show. */
  handleVisibility(hidden: boolean): void {
    if (!this.ctx) return;
    if (hidden) {
      if (this.ctx.state === 'running') {
        void this.ctx.suspend();
        this.suspendedForHidden = true;
      }
    } else if (this.ctx.state === 'suspended' && this.suspendedForHidden) {
      void this.ctx.resume();
      this.suspendedForHidden = false;
    }
  }
}

/** Shared singleton; the engine's provider owns its lifecycle. */
export const music = new MusicEngine();
