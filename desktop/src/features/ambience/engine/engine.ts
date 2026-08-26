/**
 * AmbienceEngine — owns the canvases, the frame loop, input wiring and the
 * profile. The engine is plain TypeScript with no React and no Tauri; React's
 * entire job is `new AmbienceEngine(canvas, fx, opts)` + `start()`/`stop()`.
 *
 * Lifecycle/leak contract: every listener, timer and animation frame created
 * here is released in `stop()`. React strict-mode double-mounting and the
 * 50x mount/unmount test exercise this.
 *
 * Profile split:
 *  - `calm`: terrain, sky, weather, few animals. No eggs, no buddy, no
 *    click particles. Music optional.
 *  - `full`: everything — eggs, buddy, particles, music.
 *  - `off`: no canvas, no rAF (AmbienceCanvas does not mount).
 */

import { buddyStep, createBuddy, drawBuddy, buddySnap, buddyTarget, buddyVisible, type BuddyState } from './buddy';
import { advanceClock, createClock, setWeather as clockSetWeather, type ClockState } from './clock';
import { viewToCanvas, worldViewport } from './terrain';
import { checkAchievements, journalData, loadJournalState, saveJournalState } from './eggs';
import { getCachedJournalRawSync, saveJournalRaw } from '../journalStorage';
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
  /** SFX loudness multiplier (default 1 = the prototype's fixed level). */
  soundVolume?: number;
  /** Override prefers-reduced-motion (the shell's motion setting wins). */
  reducedMotion?: boolean;
  /** Initial time of day 0..1 (default 0.3). */
  tod?: number;
  /** Day-fraction per second. Default 1/900 -> a 15-minute day. */
  todSpeed?: number;
  onEvent?: (ev: AmbienceEvent) => void;
  /**
   * Bounds the world is fitted to, in client coordinates. Defaults to the
   * whole window ({0, 0, innerWidth, innerHeight}, with the existing
   * clientWidth / 1280×720 fallbacks) — omit for the desktop behaviour.
   * Supplying this fits the world to a box instead of the viewport: the
   * fixed-width world SCALES DOWN to the box rather than cropping, and every
   * pointer coordinate is offset by bounds.left/top before it is normalised,
   * painted on the fx canvas, or converted to world units.
   */
  getBounds?: () => { left: number; top: number; w: number; h: number };
  /**
   * DOM element the world's interaction is scoped to. When provided:
   * pointermove/pointerdown bind to the host, the capture-phase clicks bind
   * to the host (so clicking elsewhere on the page cannot poke the world),
   * and a ResizeObserver refits the world when the host resizes without a
   * window resize. Omit for the desktop defaults (window/document listeners,
   * window resize/load only). The observer is disconnected in stop().
   */
  host?: HTMLElement;
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
  /** ResizeObservers on a boxed host; disconnected in stop(). */
  private resizers: ResizeObserver[] = [];
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
    this.state.soundVolume = options.soundVolume ?? 1;
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

  /** SFX loudness: 1 is the prototype's fixed level, above that plays louder. */
  setSoundVolume(v: number): void {
    this.state.soundVolume = v;
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

  /** Whether the player has a rainbow pinned via the Living Background page. */
  isRainbowPinned(): boolean {
    const world = this.state.world as WorldState | null;
    return world?.flags.rainbowPinned === true;
  }

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
   *
   * Inputs are expected in the box's own coordinate space (0,0 at the box's
   * top-left): `origin` is the canvas's offset WITHIN the box, so a
   * viewport-relative pointer must have bounds.left/top subtracted first. The
   * default box is the window at {0,0}, making that subtraction a no-op on
   * desktop.
   */
  private toCanvas(clientX: number, clientY: number): { x: number; y: number } {
    const b = this.bounds();
    const w = this.css.w || b.w || 1;
    const h = this.css.h || b.h || 1;
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

  private lastPinnedTrackId: string | null = null;

  /** Play a specific piece. Implies the player is choosing, so autoplay stops. */
  setTrack(id: string): void {
    this.lastPinnedTrackId = id;
    this.musicAuto = false;
    this.withTracks((all) => {
      const track = all.find((t) => t.id === id);
      if (track) music.start(track);
    });
  }

  /** Force an instrument for every track until the player picks another. */
  setInstrument(id: string): void {
    music.setInstrument(id, true);
  }

  /** The id of the piece currently playing, or null during silence. */
  currentTrackId(): string | null {
    return music.currentTrackId();
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
   * initial chunk.
   *
   * "Let it choose" draws from the WHOLE library through a shuffle bag (see
   * `nextTrack`). Two earlier versions both collapsed the 37-minute library
   * into a handful of pieces: the first took `[0]` of a mood and looped that
   * one track forever, and the replacement filtered to the mood of the hour,
   * which is five pieces by day (three of them Gymnopedies, which a listener
   * hears as one piece) and three at night. Neither ever reached Bumblebee,
   * Sugar Plum, Mountain King or Fate at all.
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
    // Only autoplay a random piece when "Let it choose" (musicAuto) is on.
    // If the user pinned a specific piece, resume that piece instead of
    // shuffling to a random one — otherwise toggling music off/on would
    // unexpectedly jump to a random track even though Let it decide is off.
    if (!this.musicAuto && this.lastPinnedTrackId) {
      this.withTracks((all) => {
        const pinned = all.find((t) => t.id === this.lastPinnedTrackId);
        if (pinned) music.start(pinned);
        else this.playNextAuto(all);
      });
      return;
    }
    if (!this.musicAuto) return;
    this.withTracks((all) => this.playNextAuto(all));
  }

  /** The track library, cached after the first lazy load. */
  private tracks: typeof import('./audio/tracks') | null = null;

  /**
   * Run `fn` with the track library, autoplay's end-of-piece hook attached.
   *
   * The hook used to be installed by `setMusicOn` alone, so only the path that
   * turned music on ever wired it. Pinning a piece starts playback without
   * going through there and left `onPieceEnd` null, so handing the choice back
   * to "Let it choose" afterwards had nothing listening at the loop boundary
   * and the pinned piece repeated for the rest of the session.
   */
  private withTracks(fn: (all: MusicTrack[]) => void): void {
    const run = (m: typeof import('./audio/tracks')): void => {
      this.tracks = m;
      music.onPieceEnd = () => {
        if (!this.musicAuto) return;
        this.playNextAuto(m.MUSIC_TRACKS);
      };
      fn(m.MUSIC_TRACKS);
    };
    if (this.tracks) { run(this.tracks); return; }
    void import('./audio/tracks').then(run);
  }

  /**
   * Start autoplay's next piece, unless it is the one already playing.
   *
   * The second draw covers a piece the player pinned by hand: `setTrack` does
   * not deal from the bag, so this cycle's bag can still hold the id that is
   * playing right now, and restarting it would look like the control did
   * nothing. A bag never holds an id twice, so one more draw always clears it.
   */
  private playNextAuto(all: MusicTrack[]): void {
    let next = this.nextTrack(all);
    if (next && next.id === music.currentTrackId()) next = this.nextTrack(all) ?? next;
    if (next && next.id !== music.currentTrackId()) music.start(next);
  }

  /** Autoplay's running order: a shuffled bag, refilled once it empties. */
  private musicBag: string[] = [];

  /**
   * The next piece for autoplay.
   *
   * A bag, not a fresh `Math.random()` per piece: independent draws are happy
   * to serve the same three pieces all evening, and "avoid the one that just
   * played" only rules out an immediate repeat. Dealing from a shuffled bag
   * plays every piece in the library once before any piece plays twice, which
   * is what "let it choose" is expected to mean.
   */
  private nextTrack(all: MusicTrack[]): MusicTrack | null {
    if (all.length === 0) return null;
    if (this.musicBag.length === 0) {
      const ids = all.map((t) => t.id);
      for (let i = ids.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [ids[i], ids[j]] = [ids[j], ids[i]];
      }
      // A fresh bag must not open with the piece the last one closed on —
      // the only way a bag can produce a back-to-back repeat.
      const current = music.currentTrackId();
      if (ids.length > 1 && ids[0] === current) {
        const k = 1 + Math.floor(Math.random() * (ids.length - 1));
        [ids[0], ids[k]] = [ids[k], ids[0]];
      }
      this.musicBag = ids;
    }
    const id = this.musicBag.shift();
    return all.find((t) => t.id === id) ?? null;
  }

  /**
   * Whether autoplay may rotate pieces when one ends.
   *
   * There is no separate switch for this: picking a named piece in the Living
   * Background panel turns it off, and picking "Let it choose" turns it back
   * on.
   */
  musicAuto = true;
  /**
   * Flag only, deliberately: the panel calls this on open, when its selector
   * is showing "Let it choose", to bring the engine back in line with what the
   * player is being told. Cutting a piece off mid-phrase every time somebody
   * opened the panel would be worse than letting the current one finish.
   */
  setMusicAuto(on: boolean): void { this.musicAuto = on; }

  /**
   * The player chose "Let it choose" just now: hand the running order back to
   * autoplay and move to a fresh piece immediately.
   *
   * Setting the flag alone makes the control look dead — a pinned piece loops
   * on until its next boundary, minutes away for most of this library, with
   * nothing to show the pick took. Music that is off stays off; this is a
   * choice about running order, not a play button.
   */
  shuffleNow(): void {
    this.musicAuto = true;
    if (!music.isPlaying()) return;
    this.withTracks((all) => this.playNextAuto(all));
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
    this.resizers.forEach((r) => r.disconnect());
    this.resizers = [];
    this.timers.forEach((t) => clearTimeout(t));
    this.timers = [];
    music.stop();
    this.persist();
  }

  /** Persist the journal (called on stop and after discoveries) to Agora app data. */
  private persist(): void {
    try {
      const raw = saveJournalState(this.state);
      if (raw !== null) void saveJournalRaw(raw);
    } catch {
      // journal persistence is best-effort
    }
  }

  /**
   * The box the world is fitted to — the whole window by default, matching
   * the pre-option behaviour exactly (same fallbacks, zero offset).
   */
  private bounds(): { left: number; top: number; w: number; h: number } {
    if (this.options.getBounds) return this.options.getBounds();
    return {
      left: 0,
      top: 0,
      w: window.innerWidth || document.documentElement.clientWidth || 1280,
      h: window.innerHeight || document.documentElement.clientHeight || 720,
    };
  }

  private size(): void {
    // canvas is a replaced element: inset:0 alone leaves it at intrinsic 0x0.
    // innerWidth (or a host's rect) can still be 0 on the first tick in an
    // embedded frame; fall back and re-measure so the backdrop never stays a
    // 0x0 buffer.
    const b = this.bounds();
    const vw = b.w;
    const vh = b.h;
    // The world is a fixed-size place fitted to the box, NOT a box-sized
    // one: `v.W` follows the zoom alone, and a wider box raises `v.scale`
    // instead. Widening used to widen the world itself — more hills generated
    // past the old edge, the sun's arc and every W-relative spawn stretched
    // apart — so the map grew with the window instead of being drawn larger.
    // A smaller box therefore SCALES THE WORLD DOWN — it does not crop it.
    const v = worldViewport(vw, vh, this.view.zoom);
    // Remember where the canvas actually sits: `toCanvas` has to undo this
    // offset, and reading it back off the style string every pointermove would
    // be both slower and a chance to drift out of sync. The offset is WITHIN
    // the box, so it stays box-relative whether the box is the window or a
    // position:absolute host.
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
    // are positioned from box-relative coordinates, so its canvas space has to
    // stay 1:1 with the box — grown or offset like the background, every spark
    // would draw at the wrong place. The canvas sits at the box's own origin
    // (0/0 inside a position:absolute host); the box's offset from the page is
    // the pointer handler's subtraction, not this canvas's position.
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
    // journal restore — from Agora app data (local_state.db) with WebView fallback
    let raw: string | null = null;
    try { raw = getCachedJournalRawSync(); } catch { /* ignore */ }
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
    // A host box scopes the world's interaction to itself: pointermove follows
    // the cursor only while it is over the host, and a click anywhere else on
    // the page must not poke the world. Without a host everything stays on
    // window/document exactly as before.
    const host = this.options.host ?? null;
    const on = (el: EventTarget, type: string, fn: EventListener, capture?: boolean) => {
      el.addEventListener(type, fn, capture);
      this.listeners.push({ el, type, fn, capture });
    };
    const resize = () => { this.size(); };
    on(window, 'resize', resize);
    on(window, 'load', resize);
    // A boxed host can resize without a window resize (layout shift, font
    // load, container query), so watch it directly. Disconnected in stop(),
    // keeping the mount/unmount leak contract intact.
    if (host) {
      const ro = new ResizeObserver(resize);
      ro.observe(host);
      this.resizers.push(ro);
    }

    // Pointer coordinates arrive viewport-relative (clientX/Y), but the world
    // draws into a box whose own origin is bounds.left/top. Offset once before
    // normalising or before feeding anything that paints on the fx canvas or
    // hits the world. The default bounds are {0,0} + the window, so this is a
    // no-op on desktop.
    const pointerMove = ((e: PointerEvent) => {
      const b = this.bounds();
      const x = e.clientX - b.left;
      const y = e.clientY - b.top;
      state.mx = x / (b.w || 1);
      state.my = y / (b.h || 1);
      // A gap in pointer events means the cursor was somewhere we could not see
      // it — off-window, or over another surface — and it can come back
      // anywhere. Easing from the stale position looks like the companion is
      // lost; snap it to where the cursor actually is and carry on.
      const now = performance.now();
      const gap = now - this.lastPointerMs;
      this.lastPointerMs = now;
      if (gap > 400) {
        buddySnap(this.buddy, x, y);
        this.particles.pointerJumped?.(x, y);
      }
      this.particles.pointerMoved(x, y);
      buddyTarget(this.buddy, x, y);
      {
        const p = this.toCanvas(x, y);
        state.pcx = p.x;
        state.pcy = p.y;
      }
      if (state.world) {
        const w = state.world as WorldState;
        const p = this.toCanvas(x, y);
        const m = w.hit(p.x, p.y);
        w.hover = m;
        this.bg.style.cursor = m ? 'pointer' : this.view.zoom > 1 ? 'grab' : 'default';
      }
    }) as EventListener;
    on(host ?? window, 'pointermove', pointerMove);

    // World clicks: any click that is not on an interactive control is a
    // background click, which the world owns (capture-phase so it sees the
    // click before any panel handles it; we never stopPropagation).
    const click = ((e: PointerEvent) => {
      if (Date.now() - (state.justDragged || 0) < 250) return;
      const target = e.target as Element | null;
      if (target && target.closest && target.closest(INTERACTIVE_SELECTOR)) return;
      const world = state.world as WorldState | null;
      if (!world) return;
      const b = this.bounds();
      const p = this.toCanvas(e.clientX - b.left, e.clientY - b.top);
      const hit = world.hit(p.x, p.y);
      if (hit) world.interact(hit);
      else if (world.carry) world.drop(p.x, p.y);
    }) as EventListener;
    on(host ?? document, 'click', click, true);

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
      const b = this.bounds();
      const p = this.toCanvas(e.clientX - b.left, e.clientY - b.top);
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
      const b = this.bounds();
      const w = this.css.w || b.w || 1;
      const h = this.css.h || b.h || 1;
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
    // A pan may only START inside the box; move/up/cancel stay on window so a
    // drag that leaves the host still ends cleanly.
    on(host ?? window, 'pointerdown', panDown);
    on(window, 'pointermove', panMove);
    on(window, 'pointerup', panUp);
    on(window, 'pointercancel', panUp);

    // global click burst (capture-phase, nothing can suppress it — trap 1)
    const burstClick = ((e: MouseEvent) => {
      if (this.state.reduce || this.profile === 'calm') return;
      const b = this.bounds();
      this.particles.burst(e.clientX - b.left, e.clientY - b.top, '#9FF5E2', 6, 5);
    }) as EventListener;
    on(host ?? document, 'click', burstClick, true);

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
    // time of day + weather advance with the real clock: no debug slider,
    // no weather toggle
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
