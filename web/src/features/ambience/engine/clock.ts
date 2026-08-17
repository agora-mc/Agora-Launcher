/**
 * Clock + weather — time-of-day advancement and the weather cycle, extracted
 * from the engine frame so tests can drive them deterministically.
 *
 * In the prototype, time-of-day and weather were debug controls. In the app
 * they are driven by the real clock (V5-PORT-PLAN §2): the prototype control
 * bar does not exist, so weather and time are not user toggles. The egg logic
 * that hung off the time slider (full-day, acorn-hunt) and the weather button
 * (rainbow) now hangs off this clock, and the verify-eggs port drives it the
 * same way the slider/button used to.
 */

import type { EngineState } from './state';
import type { WorldState } from './types';

export interface ClockState {
  fullDayMin: number | null;
  fullDayMax: number | null;
  acornMin: number | null;
  acornMax: number | null;
  weatherTimer: number;
}

export function createClock(): ClockState {
  return { fullDayMin: null, fullDayMax: null, acornMin: null, acornMax: null, weatherTimer: 0 };
}

/** Seconds per full weather slot; the cycle is clear → rain → clear → snow. */
const WEATHER_CYCLE = 90;

/** The weather slot index maps to a weather id: 1 = rain, 3 = snow, else clear. */
function weatherForSlot(slot: number): number {
  return slot === 1 ? 1 : slot === 3 ? 2 : 0;
}

/**
 * Advance the world clock by `dt` seconds.
 *
 * - `tod` advances by `todSpeed * dt` (default: a ~15-minute day).
 * - full-day / acorn-hunt egg tracking runs exactly as the prototype's time
 *   slider did — a sweep of `tod` through at least 0.85 of the cycle fires.
 * - the weather cycle transitions and, rain → clear during the day, spawns
 *   the rainbow (the prototype's weather-button egg path).
 *
 * `dt` and `todSpeed` are separate so tests can drive `tod` by setting it and
 * calling with dt = 0, or drive the weather by setting `clock.weatherTimer`.
 */
export function advanceClock(state: EngineState, clock: ClockState, dt: number, todSpeed = 1 / 900): void {
  // `todLocked` pins the sky where the player left it — the day stops moving,
  // but everything downstream (egg tracking, weather) still ticks.
  if (todSpeed > 0 && !state.todLocked) state.tod = (state.tod + todSpeed * dt) % 1;

  // full-day egg tracking (prototype's time-slider logic)
  let fullMin = clock.fullDayMin, fullMax = clock.fullDayMax;
  if (fullMin === null || fullMax === null) { fullMin = state.tod; fullMax = state.tod; }
  else { fullMin = Math.min(fullMin, state.tod); fullMax = Math.max(fullMax, state.tod); }
  clock.fullDayMin = fullMin; clock.fullDayMax = fullMax;
  const world = state.world as WorldState | null;
  if (world && fullMax - fullMin >= 0.85 && !world.found['full-day']) {
    world._findEgg?.('full-day');
  }

  // acorn-hunt egg: armed by three buried acorns, then a full day passes
  if (world && world.flags.acornHuntArmed && !world.found['acorn-hunt']) {
    let aMin = clock.acornMin, aMax = clock.acornMax;
    if (aMin === null || aMax === null) { aMin = state.tod; aMax = state.tod; }
    else { aMin = Math.min(aMin, state.tod); aMax = Math.max(aMax, state.tod); }
    clock.acornMin = aMin; clock.acornMax = aMax;
    if (aMax - aMin >= 0.85) world._findEgg?.('acorn-hunt');
  }

  // weather cycle — skipped while the player has pinned the weather by hand
  if (state.weatherLocked) return;
  clock.weatherTimer += dt;
  const wasRain = state.weather === 1;
  const slot = Math.floor(clock.weatherTimer / WEATHER_CYCLE) % 4;
  const next = weatherForSlot(slot);
  if (next !== state.weather) {
    state.weather = next;
    // rainbow egg: rain then sun — during the day
    if (wasRain && next === 0) {
      const w = state.world as WorldState | null;
      if (w && w.isDay()) w.spawnRainbow();
    }
  }
}

/** Force the weather to a specific id by repositioning the clock (tests). */
export function setWeather(state: EngineState, clock: ClockState, id: number): void {
  const slot = id === 1 ? 1 : id === 2 ? 3 : 0;
  clock.weatherTimer = slot * WEATHER_CYCLE + WEATHER_CYCLE / 2;
  state.weather = id;
}
