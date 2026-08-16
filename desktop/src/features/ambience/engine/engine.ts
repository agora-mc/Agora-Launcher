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

import { buddyStep, createBuddy, drawBuddy, buddySnap, buddyTarget, buddyVisible, type BuddyState } from './buddy';
import { advanceClock, createClock, setWeather as clockSetWeather, type ClockState } from './clock';
import { viewToCanvas, worldViewport } from './terrain';
import { checkAchievements, journalData, loadJournalState, saveJournalState, JOURNAL_KEY } from './eggs';
import { music } from './audio/music';
import type { MusicTrack } from './audio/tracks';
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
  /** Zoom + pan applied to the background canvas as one CSS transform. */
  private view = { zoom: 1, tx: 0, ty: 0 };
  /**
   * The background canvas's untransformed top-left, in client coordinates.
   *
   * Zero only while the canvas is exactly the viewport. Zooming OUT grows it
   * past the viewport and `size()` centres it with a negative offset — which
   * every screen→canvas conversion has to subtract back off.
   */
  private origin = { left: 0, top: 0 };
  /**
   * The background canvas element's untransformed size, in client px.
   *
   * Not the same as its buffer size in world units: the world has fixed
   * borders and is scaled to the window, so `css.w / scale === state.W`.
   */
  private css = { w: 0, h: 0 };
  /** Client px per world unit — the fixed-width world's fit to this window. */
  private scale = 1;
  /** When the last pointermove arrived; a long gap means the cursor left. */
  private lastPointerMs = 0;
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

  /**
   * Set the weather explicitly.
   *
   * Setting `state.weather` alone did nothing visible: `advanceClock` recomputes
   * the weather from its own timer on EVERY frame, so a manual choice was
   * overwritten about 16ms later. Reposition the clock instead (clock.ts already
   * exposes exactly that), and hold it there — a person who picks "Snow" in a
   * settings panel means "make it snow", not "make it snow until the cycle
   * disagrees".
   */
  setWeather(id: string): void {
    const idx = this.state.WEATHER.indexOf(id as never);
    if (idx < 0) return;
    clockSetWeather(this.state, this.clock, idx);
    this.state.weatherLocked = true;
  }

  /** Pin the weather where it is, or hand it back to the automatic cycle. */
  setWeatherLocked(on: boolean): void {
    this.state.weatherLocked = on;
  }

  /** Pin the time of day where it is, or let the day run again. */
  setTodLocked(on: boolean): void {
    this.state.todLocked = on;
  }

  /**
   * What the world's clock actually reads right now.
   *
   * Controls that mirror the clock (a time-of-day slider, the weather pills)
   * have to be told, or they drift: the day keeps turning and the cycle keeps
   * changing the weather while the panel still shows whatever was last set by
   * hand.
   */
  clockState(): { tod: number; weather: string; todLocked: boolean; weatherLocked: boolean } {
    return {
      tod: this.state.tod,
      weather: this.state.WEATHER[this.state.weather] ?? 'clear',
      todLocked: this.state.todLocked,
      weatherLocked: this.state.weatherLocked,
    };
  }

  /** Set the background zoom (0.5–2). */
  setZoom(z: number): void {
    // 0.9 floor matches the slider: below that the cover-canvas has to generate
    // a lot of extra world that is then scaled into near-invisibility.
    const next = Math.max(0.9, Math.min(2, z));
    const changedCoverage = (next < 1) !== (this.view.zoom < 1) || next < 1;
    this.view.zoom = next;
    if (next >= 1) { this.view.tx = 0; this.view.ty = 0; }
    if (changedCoverage) this.size();   // re-cover the viewport at the new scale
    this.applyView();
  }

  /** Pan the world (only meaningful while zoomed in). */
  setPan(tx: number, ty: number): void {
    this.view.tx = tx;
    this.view.ty = ty;
    this.applyView();
  }

  getView(): { zoom: number; tx: number; ty: number } { return { ...this.view }; }

  private applyView(): void {
    const { zoom, tx, ty } = this.view;
    this.bg.style.transformOrigin = '50% 50%';
    const none = zoom === 1 && tx === 0 && ty === 0;
    this.bg.style.transform = none ? '' : `translate(${tx}px, ${ty}px) scale(${zoom})`;
  }

  /**
   * Screen point → canvas point, undoing the view transform.
   *
   * The zoom is a CSS transform on the canvas, so the pixels move but the
   * canvas's own coordinate system does not. Hit tests were run against raw
   * client coordinates, which meant that as soon as you zoomed, everything was
   * clickable somewhere other than where it was drawn.
   *
   * The canvas's own offset matters just as much: at the default zoom of 0.9
   * the canvas is ~11% wider than the window and centred over it, so ignoring
   * `origin` put every hit test roughly 6% of the window's width away from the
   * cursor — the further from the middle you clicked, the further off it was.
   */
  private toCanvas(clientX: number, clientY: number): { x: number; y: number } {
    const w = this.css.w || window.innerWidth || 1;
    const h = this.css.h || window.innerHeight || 1;
    return viewToCanvas(clientX, clientY, this.view, w, h, this.origin.left, this.origin.top, this.scale);
  }

  /**
   * Put a rainbow up, or take it down.
   *
   * The world only ever spawned one as a reward for rain→sun. It is the single
   * prettiest thing in the scene, so it also deserves to be something you can
   * simply ask for — and one you asked for is PINNED: it does not fade with the
   * shower's, it stays until you take it down.
   */
  setRainbow(on: boolean): void {
    const world = this.state.world as WorldState | null;
    if (!world) return;
    world.flags.rainbowPinned = on;
    if (on) world.spawnRainbow();
    else world.clearRainbow();
  }

  /** Play a specific piece. Implies the player is choosing, so autoplay stops. */
  setTrack(id: string): void {
    this.musicAuto = false;
    void import('./audio/tracks').then((m) => {
      const track = m.MUSIC_TRACKS.find((t) => t.id === id);
      if (track) music.start(track);
    });
  }

  /** Force an instrument for every track until the player picks another. */
  setInstrument(id: string): void {
    music.setInstrument(id, true);
  }

  /** Toggle the cursor companion (buddy) on/off. */
  setBuddy(on: boolean): void {
    buddyVisible(this.buddy, on && !this.state.reduce);
  }

  /** For the High Interaction UI's drag-to-rearrange (prototype's justDragged). */
  noteDrag(): void {
    this.state.justDragged = Date.now();
  }

  /**
   * Start or stop music. tracks.ts is lazy-imported so it never lands in the
   * initial chunk (plan §6).
   *
   * Autoplay picks a RANDOM track of the current mood and moves to another when
   * the piece ends. It used to take `[0]` of the mood and then loop that one
   * track forever, so the whole 37-minute library came out as two songs: the
   * first calm piece by day and the first moody one at night. (It also asked for
   * a mood named 'bright', which no track has, so that branch silently fell
   * through to track 0.)
   */
  setMusicOn(on: boolean): void {
    if (!on) {
      music.onPieceEnd = null;
      music.stop();
      return;
    }
    // Idempotent: turning music "on" while it is already playing must not yank
    // the current piece and start another. Only the end of a piece (or an
    // explicit pick) changes the track.
    if (music.isPlaying()) return;
    void import('./audio/tracks').then((m) => {
      const pick = (): MusicTrack | null => {
        const mood = this.musicMood();
        const inMood = m.MUSIC_TRACKS.filter((t) => t.mood === mood);
        const pool = inMood.length > 0 ? inMood : m.MUSIC_TRACKS;
        if (pool.length === 0) return null;
        // Avoid repeating the piece that just finished when there is a choice.
        const current = music.currentTrackId();
        const fresh = pool.length > 1 ? pool.filter((t) => t.id !== current) : pool;
        return fresh[Math.floor(Math.random() * fresh.length)] ?? null;
      };
      music.onPieceEnd = () => {
        if (!this.musicAuto) return;
        const next = pick();
        if (next && next.id !== music.currentTrackId()) music.start(next);
      };
      const track = pick();
      if (track && track.id !== music.currentTrackId()) music.start(track);
    });
  }

  /** Mood for autoplay. Only moods that tracks actually carry. */
  private musicMood(): string {
    const world = this.state.world as WorldState | null;
    if (world && world.isNight && world.isNight()) return 'moody';
    return 'calm';
  }

  /** Whether autoplay may rotate tracks when a piece ends. */
  musicAuto = true;
  setMusicAuto(on: boolean): void { this.musicAuto = on; }

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
    const vw = window.innerWidth || document.documentElement.clientWidth || 1280;
    const vh = window.innerHeight || document.documentElement.clientHeight || 720;
    // The world is a fixed-size place fitted to the window, NOT a window-sized
    // one: `v.W` follows the zoom alone, and a wider window raises `v.scale`
    // instead. Widening used to widen the world itself — more hills generated
    // past the old edge, the sun's arc and every W-relative spawn stretched
    // apart — so the map grew with the window instead of being drawn larger.
    const v = worldViewport(vw, vh, this.view.zoom);
    // Remember where the canvas actually sits: `toCanvas` has to undo this
    // offset, and reading it back off the style string every pointermove would
    // be both slower and a chance to drift out of sync.
    this.origin.left = v.left;
    this.origin.top = v.top;
    this.css.w = v.cssW;
    this.css.h = v.cssH;
    this.scale = v.scale;
    this.bg.style.width = `${v.cssW}px`;
    this.bg.style.height = `${v.cssH}px`;
    this.bg.style.left = `${this.origin.left}px`;
    this.bg.style.top = `${this.origin.top}px`;
    // The buffer keeps the element's own pixel size — full resolution — and the
    // fit rides on a context transform instead. Shrinking the buffer to
    // WORLD_W and letting the browser stretch it would have been fewer lines
    // and a soft, resampled world on every screen wider than 1280.
    this.bg.width = v.cssW;
    this.bg.height = v.cssH;
    this.state.W = v.W;
    this.state.H = v.H;
    this.applyWorldScale();
    // The FX canvas must NOT get the cover treatment. Particles and the buddy
    // are positioned from raw client coordinates, so its canvas space has to
    // stay 1:1 with the viewport — grown or offset like the background, every
    // spark would draw at the wrong place.
    this.fx.width = vw;
    this.fx.height = vh;
    this.fx.style.width = `${vw}px`;
    this.fx.style.height = `${vh}px`;
    this.fx.style.left = '0px';
    this.fx.style.top = '0px';
  }

  /**
   * Point the background context at world units.
   *
   * Assigning `canvas.width`/`height` resets the context — transform included —
   * so this has to run after every resize. It also runs at the top of every
   * frame: it costs one call, and a drawing routine that ever left the save
   * stack unbalanced would otherwise silently shift the entire world.
   *
   * The FX canvas is left alone on purpose. Particles and the buddy are placed
   * from raw client coordinates, so their context stays 1:1 with the viewport.
   */
  private applyWorldScale(): void {
    this.bgCtx.setTransform(this.scale, 0, 0, this.scale, 0, 0);
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
      // A gap in pointer events means the cursor was somewhere we could not see
      // it — off-window, or over another surface — and it can come back
      // anywhere. Easing from the stale position looks like the companion is
      // lost; snap it to where the cursor actually is and carry on.
      const now = performance.now();
      const gap = now - this.lastPointerMs;
      this.lastPointerMs = now;
      if (gap > 400) {
        buddySnap(this.buddy, e.clientX, e.clientY);
        this.particles.pointerJumped?.(e.clientX, e.clientY);
      }
      this.particles.pointerMoved(e.clientX, e.clientY);
      buddyTarget(this.buddy, e.clientX, e.clientY);
      {
        const p = this.toCanvas(e.clientX, e.clientY);
        state.pcx = p.x;
        state.pcy = p.y;
      }
      if (state.world) {
        const w = state.world as WorldState;
        const p = this.toCanvas(e.clientX, e.clientY);
        const m = w.hit(p.x, p.y);
        w.hover = m;
        this.bg.style.cursor = m ? 'pointer' : this.view.zoom > 1 ? 'grab' : 'default';
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
      const p = this.toCanvas(e.clientX, e.clientY);
      const hit = world.hit(p.x, p.y);
      if (hit) world.interact(hit);
      else if (world.carry) world.drop(p.x, p.y);
    }) as EventListener;
    on(document, 'click', click, true);

    /**
     * Drag to pan, but only while zoomed in and only when the press did not
     * land on something interactive. Zooming into a corner of the world is
     * useless if you cannot then move around it; and a drag that started on a
     * prop or a UI control must stay that interaction, not become a pan.
     */
    let dragging: { x: number; y: number; tx: number; ty: number } | null = null;
    const panDown = ((e: PointerEvent) => {
      if (this.view.zoom <= 1 || e.button !== 0) return;
      const target = e.target as Element | null;
      if (target && target.closest && target.closest(INTERACTIVE_SELECTOR)) return;
      const world = state.world as WorldState | null;
      const p = this.toCanvas(e.clientX, e.clientY);
      if (world && world.hit(p.x, p.y)) return;   // that press belongs to the prop
      dragging = { x: e.clientX, y: e.clientY, tx: this.view.tx, ty: this.view.ty };
      this.bg.style.cursor = 'grabbing';
    }) as EventListener;
    const panMove = ((e: PointerEvent) => {
      if (!dragging) return;
      const dx = e.clientX - dragging.x;
      const dy = e.clientY - dragging.y;
      // Bound the pan so the world can never be dragged fully off-screen. The
      // pan is client px, so it bounds against the ELEMENT, not the world units
      // drawn inside it.
      const w = this.css.w || window.innerWidth || 1;
      const h = this.css.h || window.innerHeight || 1;
      const maxX = (w * (this.view.zoom - 1)) / 2;
      const maxY = (h * (this.view.zoom - 1)) / 2;
      this.setPan(
        Math.max(-maxX, Math.min(maxX, dragging.tx + dx)),
        Math.max(-maxY, Math.min(maxY, dragging.ty + dy)),
      );
      // Suppress the click that ends a real drag, so panning never also pokes
      // whatever ended up under the cursor.
      if (Math.abs(dx) + Math.abs(dy) > 4) state.justDragged = Date.now();
    }) as EventListener;
    const panUp = (() => {
      if (!dragging) return;
      dragging = null;
      this.bg.style.cursor = this.view.zoom > 1 ? 'grab' : 'default';
    }) as EventListener;
    on(window, 'pointerdown', panDown);
    on(window, 'pointermove', panMove);
    on(window, 'pointerup', panUp);
    on(window, 'pointercancel', panUp);

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

    this.applyWorldScale();
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
