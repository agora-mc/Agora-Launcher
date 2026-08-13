/**
 * AmbienceEngine — owns the canvases, the frame loop, input wiring and the
 * profile. The engine is plain TypeScript with no React and no Tauri; React's
 * entire job is `new AmbienceEngine(canvas, fx, opts)` + `start()`/`stop()`.
 *
 * Lifecycle/leak contract (V5-PORT-PLAN trap 9): every listener, timer and
 * animation frame created here is released in `stop()`. React strict-mode
 * double-mounting and the 50x mount/unmount test exercise this.
 *
 * Profile split (plan §2 / §10):
 *  - `calm`: terrain, sky, weather, few animals. No eggs, no buddy, no
 *    click particles. Music optional.
 *  - `full`: everything — eggs, buddy, particles, music.
 *  - `off`: no canvas, no rAF (AmbienceCanvas does not mount).
 */

import { buddyStep, createBuddy, drawBuddy, buddyTarget, buddyVisible, type BuddyState } from './buddy';
import { advanceClock, createClock, type ClockState } from './clock';
import { checkAchievements, journalData, loadJournalState, saveJournalState, JOURNAL_KEY } from './eggs';
import { music } from './audio/music';
import { blip as sfxBlip, resumeSfx } from './audio/sfx';
import { ParticleLayer } from './particles';
import { createEngineState, type AmbienceEvent, type EngineState } from './state';
import { skyFrame } from './sky';
import type { WorldState } from './types';
import { createWorld } from './world';

export type AmbienceProfile = 'off' | 'calm' | 'full';

export interface AmbienceEngineOptions {
  profile?: AmbienceProfile;
  /** Music volume 0..1 (default 0.35, the prototype's default). */
  musicVolume?: number;
  musicOn?: boolean;
  /** Override prefers-reduced-motion (the shell's motion setting wins). */
  reducedMotion?: boolean;
  /** Initial time of day 0..1 (default 0.3). */
  tod?: number;
  /** Day-fraction per second. Default 1/900 -> a 15-minute day. */
  todSpeed?: number;
  onEvent?: (ev: AmbienceEvent) => void;
}

/** Emitted once the world is built (React can open the Field Journal). */
export interface AmbienceReady {
  world: WorldState;
  journal: ReturnType<typeof journalData>;
}

const INTERACTIVE_SELECTOR = [
  'button', 'a', 'input', 'select', 'textarea',
  '[role="button"]', '[role="link"]', '[role="menuitem"]', '[role="tab"]', '[role="checkbox"]',
  '[role="switch"]', '[role="option"]', '[role="radio"]', '[contenteditable]', '[data-radix-dialog-content]',
].join(',');

export class AmbienceEngine {
  readonly state: EngineState;
  private bg: HTMLCanvasElement;
  private fx: HTMLCanvasElement;
  private bgCtx: CanvasRenderingContext2D;
  private fxCtx: CanvasRenderingContext2D;
  private particles = new ParticleLayer();
  private buddy: BuddyState;
  private profile: AmbienceProfile;
  private options: AmbienceEngineOptions;
  private running = false;
  private raf = 0;
  private blinkUntil = 0;
  private listeners: Array<{ el: EventTarget; type: string; fn: EventListener; capture?: boolean }> = [];
  private timers: number[] = [];
  private clock: ClockState = createClock();

  constructor(bg: HTMLCanvasElement, fx: HTMLCanvasElement, options: AmbienceEngineOptions = {}) {
    this.bg = bg;
    this.fx = fx;
    this.bgCtx = bg.getContext('2d') as CanvasRenderingContext2D;
    this.fxCtx = fx.getContext('2d') as CanvasRenderingContext2D;
    this.options = options;
    this.state = createEngineState(options.reducedMotion);
    this.state.tod = options.tod ?? 0.3;
    this.state.soundOn = false;
    this.profile = options.profile ?? 'full';
    this.particles.reduced = this.state.reduce;
    this.buddy = createBuddy(this.state.reduce, 0, 0);
  }

  get profileValue(): AmbienceProfile { return this.profile; }

  getWorld(): WorldState | null {
    return this.state.world as WorldState | null;
  }

  journal(): ReturnType<typeof journalData> {
    return journalData(this.state);
  }

  /** Set the ambient profile live. */
  setProfile(p: AmbienceProfile): void {
    this.profile = p;
    this.particles.reduced = this.state.reduce || p === 'calm';
    buddyVisible(this.buddy, p === 'full' && !this.state.reduce);
    this.bg.style.cursor = p === 'full' ? 'default' : 'default';
  }

  setSoundOn(on: boolean): void {
    this.state.soundOn = on;
  }

  /** For the High Interaction UI's drag-to-rearrange (prototype's justDragged). */
  noteDrag(): void {
    this.state.justDragged = Date.now();
  }

  /** Start music. tracks.ts is lazy-imported so it never lands in the initial chunk (plan §6). */
  setMusicOn(on: boolean): void {
    if (on) {
      const world = this.state.world as WorldState | null;
      const mood = world && world.isNight && world.isNight()
        ? 'moody'
        : world && world.found && world.found['complete'] ? 'bright' : 'calm';
      void import('./audio/tracks').then((m) => {
        const want = m.MUSIC_TRACKS.filter((t) => t.mood === mood)[0];
        const track = want || m.MUSIC_TRACKS[0];
        if (track && track.id !== music.currentTrackId()) music.start(track);
      });
    } else {
      music.stop();
    }
  }

  setMusicVolume(v: number): void {
    music.setVolume(v);
  }

  /** Advance / set the time of day (0..1). */
  setTod(t: number): void {
    this.state.tod = t;
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    this.size();
    if (!this.state.world) this.buildWorld();
    this.attach();
    // music may start only after a user gesture; the shell calls
    // `engine.unlockAudio()` from a pointerdown to begin playback.
    if (this.options.musicOn) {
      const kick = () => {
        this.setMusicOn(true);
        window.removeEventListener('pointerdown', kick);
      };
      window.addEventListener('pointerdown', kick);
      this.listeners.push({ el: window, type: 'pointerdown', fn: kick });
    }
    this.raf = requestAnimationFrame(this.frame);
  }

  /** Resume the shared AudioContext after a user gesture (browser autoplay). */
  unlockAudio(): void {
    resumeSfx();
    if (this.options.musicOn) this.setMusicOn(true);
  }

  stop(): void {
    if (!this.running) return;
    this.running = false;
    cancelAnimationFrame(this.raf);
    this.raf = 0;
    this.listeners.forEach(({ el, type, fn, capture }) => el.removeEventListener(type, fn, capture));
    this.listeners = [];
    this.timers.forEach((t) => clearTimeout(t));
    this.timers = [];
    music.stop();
    this.persist();
  }

  /** Persist the journal (called on stop and after discoveries). */
  private persist(): void {
    try {
      const raw = saveJournalState(this.state);
      if (raw !== null) window.localStorage.setItem(JOURNAL_KEY, raw);
    } catch {
      // journal persistence is best-effort
    }
  }

  private size(): void {
    // canvas is a replaced element: inset:0 alone leaves it at intrinsic 0x0.
    // innerWidth can still be 0 on the first tick in an embedded frame; fall
    // back and re-measure so the backdrop never stays a 0x0 buffer.
    this.state.W = this.bg.width = this.fx.width = window.innerWidth || document.documentElement.clientWidth || 1280;
    this.state.H = this.bg.height = this.fx.height = window.innerHeight || document.documentElement.clientHeight || 720;
  }

  private buildWorld(): void {
    const state = this.state;
    const emit = (ev: AmbienceEvent) => {
      if (this.profile === 'calm' && (ev.type === 'discovery' || ev.type === 'achievement')) return;
      if (ev.type === 'discovery') {
        music.duck();
      }
      this.options.onEvent?.(ev);
      if (ev.type === 'discovery' || ev.type === 'achievement' || ev.type === 'completion') this.persist();
    };
    const world = createWorld(state, {
      blip: (f, d, t, v) => this.blip(f, d, t, v),
      burst: (x, y, c, n, s) => this.particles.burst(x, y, c, n, s),
      emit,
    });
    state.world = world;
    // journal restore
    let raw: string | null = null;
    try { raw = window.localStorage.getItem(JOURNAL_KEY); } catch { /* ignore */ }
    const saved = loadJournalState(state, raw);
    if (saved) {
      (saved.unlocked || []).forEach((name) => { state.unlocked[name] = true; });
      checkAchievements(state, emit);
    }
  }

  private blip(f: number, d?: number, t?: OscillatorType, v?: number): void {
    // SFX routed through the shared audio; respects the engine's soundOn flag
    if (!this.state.soundOn) return;
    sfxBlip(this.state, f, d, t, v);
  }

  private attach(): void {
    const state = this.state;
    const on = (el: EventTarget, type: string, fn: EventListener, capture?: boolean) => {
      el.addEventListener(type, fn, capture);
      this.listeners.push({ el, type, fn, capture });
    };
    const resize = () => { this.size(); };
    on(window, 'resize', resize);
    on(window, 'load', resize);

    const pointerMove = ((e: PointerEvent) => {
      state.mx = e.clientX / (window.innerWidth || 1);
      state.my = e.clientY / (window.innerHeight || 1);
      this.particles.pointerMoved(e.clientX, e.clientY);
      buddyTarget(this.buddy, e.clientX, e.clientY);
      if (state.world) {
        const w = state.world as WorldState;
        const m = w.hit(e.clientX, e.clientY);
        w.hover = m;
        this.bg.style.cursor = m ? 'pointer' : 'default';
      }
    }) as EventListener;
    on(window, 'pointermove', pointerMove);

    // World clicks: any click that is not on an interactive control is a
    // background click, which the world owns (capture-phase so it sees the
    // click before any panel handles it; we never stopPropagation).
    const click = ((e: PointerEvent) => {
      if (Date.now() - (state.justDragged || 0) < 250) return;
      const target = e.target as Element | null;
      if (target && target.closest && target.closest(INTERACTIVE_SELECTOR)) return;
      const world = state.world as WorldState | null;
      if (!world) return;
      const hit = world.hit(e.clientX, e.clientY);
      if (hit) world.interact(hit);
      else if (world.carry) world.drop(e.clientX, e.clientY);
    }) as EventListener;
    on(document, 'click', click, true);

    // global click burst (capture-phase, nothing can suppress it — trap 1)
    const burstClick = ((e: MouseEvent) => {
      if (this.state.reduce || this.profile === 'calm') return;
      this.particles.burst(e.clientX, e.clientY, '#9FF5E2', 6, 5);
    }) as EventListener;
    on(document, 'click', burstClick, true);

    // blink timer for the buddy (prototype's setInterval(4200))
    const blinkTimer = window.setInterval(() => {
      if (this.buddy.on && !this.state.reduce) this.blinkUntil = performance.now() + 130;
    }, 4200);
    this.timers.push(blinkTimer);

    // visibility: pause everything while hidden (trap 2)
    const vis = () => music.handleVisibility(document.hidden);
    on(document, 'visibilitychange', vis);
  }

  private frame = (ts: number): void => {
    if (!this.running) return;
    const state = this.state;
    // time of day + weather advance with the real clock (plan §2: no debug
    // slider, no weather toggle)
    const todSpeed = this.options.todSpeed ?? 1 / 900;
    advanceClock(state, this.clock, 0.016, todSpeed);

    skyFrame(state, this.bgCtx, ts);
    this.particles.frame(this.fxCtx);

    // buddy (drawn on the fx canvas, moved by the same loop)
    buddyStep(this.buddy, ts);
    if (this.buddy.on && !state.reduce) {
      drawBuddy(this.fxCtx, this.buddy, ts < this.blinkUntil);
    }
    this.raf = requestAnimationFrame(this.frame);
  };
}
