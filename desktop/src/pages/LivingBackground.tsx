/**
 * Living Background — a full, unobstructed view of the living world so you
 * can actually see it and play with it. The normal shell (page background +
 * blur) is dropped while this page is open; the ambience canvas shows through
 * edge to edge, and the controls below are the same ones the prototype's
 * "@top control bar" exposed (sound, weather, time, music, volume) plus a few
 * extras (zoom, cursor companion).
 *
 * It only appears in the sidebar while the living background is enabled.
 */

import { useEffect, useRef, useState } from 'react';
import { Mountain, Music, Volume2, CloudRain, CloudSnow, Sun, ZoomIn, ZoomOut, Sparkles, Rainbow, ChevronDown, ChevronRight, Lock, LockOpen } from 'lucide-react';
import { MUSIC_TRACK_CHOICES, INSTRUMENT_CHOICES } from '../features/ambience/engine/audio/trackChoices';
import { useAmbience } from '../features/ambience/AmbienceProvider';

/**
 * "Leave this alone" for one of the world's own clocks. Pressed, the day stops
 * turning / the weather stops cycling and stays exactly where it is.
 */
function LockButton({ locked, label, onClick }: { locked: boolean; label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={locked}
      aria-label={label}
      title={locked ? 'Locked — the world will not change this on its own' : 'Lock this where it is now'}
      className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-semibold ${locked
          ? 'border-primary bg-primary text-primary-foreground'
          : 'border-border bg-muted text-muted-foreground hover:text-foreground'
        }`}
    >
      {locked ? <Lock className="h-3 w-3" aria-hidden="true" /> : <LockOpen className="h-3 w-3" aria-hidden="true" />}
      {locked ? 'Locked' : 'Lock'}
    </button>
  );
}

export function LivingBackground() {
  const {
    soundOn,
    setSoundOn,
    soundVolume,
    setSoundVolume,
    musicVolume,
    setMusicVolume,
    musicOn,
    setMusicOn,
    clearBackground,
    setClearBackground,
    setTod,
    setWeather,
    setTodLocked,
    setWeatherLocked,
    readClock,
    setZoom,
    setRainbow,
    setTrack,
    setInstrument,
    setMusicAuto,
    shuffleMusic,
    setBuddy,
    ready,
  } = useAmbience();

  const [time, setTime] = useState(0.3);
  // Fifth tick of the old range (0.5 + 4×0.1) is now BOTH the minimum and the
  // default: below this the world was scaled so far down that the extra terrain
  // generated to cover the viewport was mostly wasted, and the props got tiny.
  const MIN_ZOOM = 0.9;
  const DEFAULT_ZOOM = MIN_ZOOM;
  const [zoom, setZoomValue] = useState(DEFAULT_ZOOM);
  const [buddy, setBuddyValue] = useState(true);
  const [weather, setWeatherValue] = useState<'clear' | 'rain' | 'snow'>('clear');
  const [todLocked, setTodLockedValue] = useState(false);
  const [weatherLocked, setWeatherLockedValue] = useState(false);
  const [rainbow, setRainbowValue] = useState(false);
  const [track, setTrackValue] = useState('');
  const [instrument, setInstrumentValue] = useState('');
  // The panel covers a good part of the world it is configuring, so it folds
  // away — but the reopen control stays put, never hunted for.
  const [panelOpen, setPanelOpen] = useState(true);

  // Announce the page to the engine (companion on).
  useEffect(() => {
    setBuddy(buddy);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /**
   * Keep the engine in step with what the piece selector is showing.
   *
   * The selector opens on "Let it choose" every time the panel mounts, but a
   * piece pinned earlier in the session is still pinned in the engine — and
   * re-picking the option React already has selected fires no change event, so
   * the player had no way to say so. Autoplay resumes at the end of the piece
   * that is running; this does not cut it off (see the engine's setMusicAuto).
   */
  useEffect(() => {
    if (track === '') setMusicAuto(true);
  }, [track, setMusicAuto]);

  /**
   * Mirror the world's own clock.
   *
   * The day keeps turning and the weather keeps cycling while this panel is
   * open, so a slider that only ever moved when dragged was wrong within
   * seconds of being set — it showed the last thing you asked for, not the time
   * it actually is out there. Read the engine back instead. `lastTouch` yields
   * to a drag in progress so the poll cannot fight your thumb.
   */
  const lastTouch = useRef(0);
  const readClockRef = useRef(readClock);
  readClockRef.current = readClock;
  useEffect(() => {
    const id = window.setInterval(() => {
      const c = readClockRef.current();
      if (!c) return;
      setWeatherValue(c.weather);
      setWeatherLockedValue(c.weatherLocked);
      setTodLockedValue(c.todLocked);
      if (Date.now() - lastTouch.current > 1000) setTime(Math.round(c.tod * 100) / 100);
    }, 500);
    return () => window.clearInterval(id);
  }, []);

  const changeTime = (t: number) => {
    lastTouch.current = Date.now();
    setTime(t);
    setTod(t);
  };
  const changeWeather = (w: 'clear' | 'rain' | 'snow') => {
    setWeatherValue(w);
    setWeather(w);
    // Asking for a weather by hand pins it — otherwise the cycle would undo the
    // choice within a frame. Reflect that here so the lock reads true.
    setWeatherLockedValue(true);
  };
  const toggleTodLock = () => {
    const next = !todLocked;
    setTodLockedValue(next);
    setTodLocked(next);
  };
  const toggleWeatherLock = () => {
    const next = !weatherLocked;
    setWeatherLockedValue(next);
    setWeatherLocked(next);
  };
  // Push the default down to the engine as soon as it exists.
  useEffect(() => {
    if (ready) setZoom(DEFAULT_ZOOM);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready]);

  const changeZoom = (z: number) => {
    const clamped = Math.max(MIN_ZOOM, Math.min(2, Math.round(z * 10) / 10));
    setZoomValue(clamped);
    setZoom(clamped);
  };
  const toggleBuddy = () => {
    const next = !buddy;
    setBuddyValue(next);
    setBuddy(next);
  };

  return (
    <div className="space-y-6" data-testid="living-background-page">
      {/* Collapsed, the page gets out of the way entirely — no title, no
          description, no panel chrome — because the whole point of this screen is
          to look at the world. Only the reopen button stays. */}
      {panelOpen ? (
        <header className="flex flex-wrap items-end justify-between gap-4">
          <div className="agora-hero compact">
            <h2 className="text-2xl font-bold">Living Background</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              The full world, unobstructed. Poke the hills, wait for night, change the weather — everything here
              is decorative and never touches your game.
            </p>
          </div>
        </header>
      ) : (
        <button
          type="button"
          onClick={() => setPanelOpen(true)}
          aria-expanded={false}
          aria-controls="living-bg-panel-body"
          className="inline-flex items-center gap-2 rounded-full border border-primary/30 bg-background/80 px-4 py-2 text-sm font-semibold shadow-lg backdrop-blur-md hover:bg-background"
          data-testid="living-bg-reopen"
        >
          <ChevronRight className="h-4 w-4" aria-hidden="true" />
          Show controls
        </button>
      )}

      {/* The control panel — the same set as the prototype's top control bar,
          plus extras (zoom, cursor companion). Floating panel over the world
          so it reads as a proper control deck, not a buried settings block. */}
      {panelOpen && (
        <section
          className="rounded-2xl border border-border bg-card/85 p-4 shadow-lg backdrop-blur-md"
          aria-label="Living background controls"
          data-testid="living-bg-controls"
        >
          <button
            type="button"
            onClick={() => setPanelOpen((v) => !v)}
            aria-expanded={panelOpen}
            aria-controls="living-bg-panel-body"
            className="mb-3 flex w-full items-center gap-2 text-xs font-bold uppercase tracking-[0.2em] text-muted-foreground hover:text-foreground"
          >
            <ChevronDown className="h-4 w-4" aria-hidden="true" />
            Control panel
            <span className="ml-auto font-normal normal-case tracking-normal text-muted-foreground">Hide</span>
          </button>
          <div id="living-bg-panel-body" className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            <div className="space-y-1 text-sm">
              <label className="flex items-center gap-3">
                <input
                  type="checkbox"
                  aria-label="Ambient sounds"
                  checked={soundOn}
                  onChange={(e) => setSoundOn(e.target.checked)}
                  className="h-5 w-5 accent-primary"
                />
                <span className="inline-flex items-center gap-2">
                  <Volume2 className="h-4 w-4" aria-hidden="true" />
                  Ambient sounds
                </span>
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="range"
                  aria-label="Ambient sound volume"
                  min="0"
                  max="100"
                  step="5"
                  disabled={!soundOn}
                  value={Math.round(soundVolume * 100)}
                  onChange={(e) => setSoundVolume(Number(e.target.value) / 100)}
                  className="w-full accent-primary disabled:opacity-40"
                />
                <div><span className="w-10 text-right text-xs text-muted-foreground">{Math.round(soundVolume * 100)}%</span></div>
              </div>
            </div>

            <div className="space-y-1 text-sm">
              <span className="flex items-center gap-2 font-medium">
                <Music className="h-4 w-4" aria-hidden="true" />
                Music
                <button
                  type="button"
                  onClick={() => setMusicOn(!musicOn)}
                  aria-pressed={musicOn}
                  aria-label="Toggle music"
                  className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-semibold ${musicOn
                      ? 'border-primary bg-primary text-primary-foreground'
                      : 'border-border bg-muted text-muted-foreground hover:text-foreground'
                    }`}
                >
                  {musicOn ? 'Playing' : 'Muted'}
                </button>
              </span>
              <div className="flex items-center gap-2">
                <input
                  type="range"
                  aria-label="Music volume"
                  min="0"
                  max="100"
                  step="5"
                  disabled={!musicOn}
                  value={Math.round(musicVolume * 100)}
                  onChange={(e) => setMusicVolume(Number(e.target.value) / 100)}
                  className="w-full accent-primary disabled:opacity-40"
                />
                <div><span className="w-10 text-right text-xs text-muted-foreground">{Math.round(musicVolume * 100)}%</span></div>
              </div>
            </div>

            {/* Not a <label>: the lock is a button, and interactive content
              inside a label makes the label click through to the slider. */}
            <div className="space-y-1 text-sm">
              <span className="flex items-center gap-2 font-medium">
                <Sun className="h-4 w-4" aria-hidden="true" />
                Time of day
                <LockButton locked={todLocked} label="Lock time of day" onClick={toggleTodLock} />
              </span>
              <input
                type="range"
                aria-label="Time of day"
                min="0"
                max="1"
                step="0.01"
                value={time}
                onChange={(e) => changeTime(Number(e.target.value))}
                className="w-full accent-primary"
              />
              <span className="block text-xs text-muted-foreground">
                {time < 0.25 ? 'Deep night' : time < 0.45 ? 'Dawn' : time < 0.7 ? 'Day' : 'Dusk'}
                {todLocked ? ' · held' : ''}
              </span>
            </div>

            <div className="space-y-1 text-sm">
              <span className="flex items-center gap-2 font-medium">
                <CloudRain className="h-4 w-4" aria-hidden="true" />
                Weather
                <LockButton locked={weatherLocked} label="Lock weather" onClick={toggleWeatherLock} />
              </span>
              <div className="flex flex-wrap gap-1.5">
                {(['clear', 'rain', 'snow'] as const).map((w) => (
                  <button
                    key={w}
                    type="button"
                    onClick={() => changeWeather(w)}
                    aria-pressed={weather === w}
                    className={`rounded-full border px-3 py-1 text-xs font-semibold capitalize ${weather === w ? 'border-primary bg-primary text-primary-foreground' : 'border-border bg-muted text-foreground hover:bg-muted'}`}
                  >
                    {w === 'clear' ? <CloudRain className="mr-1 inline h-3 w-3" /> : w === 'rain' ? <CloudRain className="mr-1 inline h-3 w-3" /> : <CloudSnow className="mr-1 inline h-3 w-3" />}
                    {w}
                  </button>
                ))}
              </div>
            </div>

            <div className="space-y-1 text-sm">
              <span className="inline-flex items-center gap-2 font-medium">
                <Mountain className="h-4 w-4" aria-hidden="true" />
                Zoom
              </span>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => changeZoom(zoom - 0.1)}
                  aria-label="Zoom out"
                  className="rounded-lg border border-input bg-muted p-1.5 text-foreground hover:bg-muted"
                >
                  <ZoomOut className="h-4 w-4" aria-hidden="true" />
                </button>
                <input
                  type="range"
                  aria-label="Zoom"
                  min={MIN_ZOOM}
                  max="2"
                  step="0.1"
                  value={zoom}
                  onChange={(e) => changeZoom(Number(e.target.value))}
                  className="w-full accent-primary"
                />
                <button
                  type="button"
                  onClick={() => changeZoom(zoom + 0.1)}
                  aria-label="Zoom in"
                  className="rounded-lg border border-input bg-muted p-1.5 text-foreground hover:bg-muted"
                >
                  <ZoomIn className="h-4 w-4" aria-hidden="true" />
                </button>
              </div>
            </div>

            <label className="flex items-center gap-3 text-sm">
              <input
                type="checkbox"
                aria-label="Cursor companion"
                checked={buddy}
                onChange={toggleBuddy}
                className="h-5 w-5 accent-primary"
              />
              <span className="inline-flex items-center gap-2">
                <Sparkles className="h-4 w-4" aria-hidden="true" />
                Cursor companion
              </span>
            </label>

            <div>
              <label className="flex items-start gap-3 text-sm">
                <input
                  type="checkbox"
                  aria-label="Rainbow"
                  checked={rainbow}
                  onChange={(e) => { setRainbowValue(e.target.checked); setRainbow(e.target.checked); }}
                  className="mt-0.5 h-5 w-5 accent-primary"
                />
                <span>
                  <span className="inline-flex items-center gap-2 font-medium">
                    <Rainbow className="h-4 w-4" aria-hidden="true" />
                    Rainbow
                  </span>
                  <span className="block text-xs text-muted-foreground">Keeps one up. A rainstorm's own fades after a few seconds.</span>
                </span>
              </label>
              <br></br>
              <label className="flex items-start gap-3 text-sm">
                <input
                  type="checkbox"
                  aria-label="Hide the standard background"
                  checked={clearBackground}
                  onChange={(e) => setClearBackground(e.target.checked)}
                  className="mt-0.5 h-5 w-5 accent-primary"
                />
                <span>
                  <span className="block font-medium">Hide the standard background</span>
                  <span className="block text-xs text-muted-foreground">Page background to 0% opacity behind the world.</span>
                </span>
              </label>
            </div>

            <label className="space-y-1 text-sm">
              <span className="font-medium">Music piece</span>
              <span className="block text-xs text-muted-foreground">
                Let it choose shuffles the whole library and moves on when a piece ends. Naming a piece keeps that one playing.
              </span>
              <select
                aria-label="Music piece"
                value={track}
                onChange={(e) => {
                  setTrackValue(e.target.value);
                  // "Let it choose" IS autoplay -- there is no separate switch.
                  if (e.target.value) setTrack(e.target.value);
                  else shuffleMusic();
                }}
                className="block w-full rounded-lg border border-input bg-background px-3 py-2 text-sm text-foreground"
              >
                <option value="">Let it choose</option>
                {MUSIC_TRACK_CHOICES.map((t) => (
                  <option key={t.id} value={t.id}>{t.name} · {t.mood}</option>
                ))}
              </select>
            </label>

            <label className="space-y-1 text-sm">
              <span className="font-medium">Instrument</span>
              <select
                aria-label="Instrument"
                value={instrument}
                onChange={(e) => { setInstrumentValue(e.target.value); if (e.target.value) setInstrument(e.target.value); }}
                className="block w-full rounded-lg border border-input bg-background px-3 py-2 text-sm text-foreground"
              >
                <option value="">Suits the piece</option>
                {INSTRUMENT_CHOICES.map((i) => (
                  <option key={i.id} value={i.id}>{i.name}</option>
                ))}
              </select>
            </label>


          </div>
        </section>
      )}
    </div>
  );
}
