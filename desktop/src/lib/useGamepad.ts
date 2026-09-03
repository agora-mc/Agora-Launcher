import { useEffect, useRef, useState } from 'react';

export type GamepadDirection = 'up' | 'down' | 'left' | 'right';
export type GamepadButton = 'a' | 'b' | 'start';
export type GamepadIntent =
  | { type: 'direction'; direction: GamepadDirection }
  | { type: 'button'; button: GamepadButton };

export interface UseGamepadOptions {
  /** Called for each debounced direction or rising-edge button intent. */
  onIntent?: (intent: GamepadIntent) => void;
  /** Called only when the presence of at least one gamepad changes. */
  onConnectionChange?: (connected: boolean) => void;
  initialRepeatDelayMs?: number;
  repeatIntervalMs?: number;
}

export interface UseGamepadResult {
  connected: boolean;
  gamepadCount: number;
}

const DEFAULT_INITIAL_REPEAT_DELAY_MS = 420;
const DEFAULT_REPEAT_INTERVAL_MS = 180;
const AXIS_DEADZONE = 0.55;

function connectedGamepads(): Gamepad[] {
  if (typeof navigator === 'undefined' || typeof navigator.getGamepads !== 'function') {
    return [];
  }

  try {
    // GamepadList is sparse in real browsers. Array.from preserves the holes
    // as null values, which are intentionally filtered here.
    return Array.from(navigator.getGamepads() ?? []).filter(
      (gamepad): gamepad is Gamepad => gamepad != null && gamepad.connected !== false,
    );
  } catch {
    // A browser can revoke the gamepad list while a device is disconnecting.
    return [];
  }
}

function pressed(gamepad: Gamepad, index: number): boolean {
  return gamepad.buttons[index]?.pressed === true;
}

function axisDirection(gamepad: Gamepad): GamepadDirection | null {
  const x = gamepad.axes[0] ?? 0;
  const y = gamepad.axes[1] ?? 0;
  if (Math.max(Math.abs(x), Math.abs(y)) < AXIS_DEADZONE) return null;
  if (Math.abs(x) >= Math.abs(y)) return x < 0 ? 'left' : 'right';
  return y < 0 ? 'up' : 'down';
}

function dpadDirection(gamepad: Gamepad): GamepadDirection | null {
  if (pressed(gamepad, 12)) return 'up';
  if (pressed(gamepad, 13)) return 'down';
  if (pressed(gamepad, 14)) return 'left';
  if (pressed(gamepad, 15)) return 'right';
  return null;
}

function directionFor(gamepad: Gamepad): GamepadDirection | null {
  // Prefer the physical D-pad when both controls are held. Either control can
  // still drive navigation when used on its own.
  return dpadDirection(gamepad) ?? axisDirection(gamepad);
}

function buttonIntents(gamepad: Gamepad, previous: boolean[]): GamepadIntent[] {
  const buttons: Array<[number, GamepadButton]> = [
    [0, 'a'],
    [1, 'b'],
    [9, 'start'],
  ];
  const intents: GamepadIntent[] = [];
  for (const [index, button] of buttons) {
    const isPressed = pressed(gamepad, index);
    if (isPressed && !previous[index]) {
      intents.push({ type: 'button', button });
    }
    previous[index] = isPressed;
  }
  return intents;
}

/**
 * Read controller presence and input using the Web Gamepad API.
 *
 * Presence is event-driven (`gamepadconnected` / `gamepaddisconnected`) and
 * always live. Input is not: the API fires no events for axes, so buttons and
 * sticks have to be polled on requestAnimationFrame — and that polling starts
 * only once a pad is actually present, because there is nothing to read
 * before then. That is why this needs no on/off switch. A user who never
 * touches a controller never starts an animation-frame loop, which would
 * otherwise keep the renderer awake forever for everyone — a poor trade on
 * the battery-powered handhelds this feature exists for.
 *
 * Directional input fires immediately, then repeats after a key-like delay
 * while the D-pad or left stick remains held.
 */
export function useGamepad({
  onIntent,
  onConnectionChange,
  initialRepeatDelayMs = DEFAULT_INITIAL_REPEAT_DELAY_MS,
  repeatIntervalMs = DEFAULT_REPEAT_INTERVAL_MS,
}: UseGamepadOptions = {}): UseGamepadResult {
  const [gamepadCount, setGamepadCount] = useState(0);
  const intentRef = useRef(onIntent);
  const connectionRef = useRef(onConnectionChange);

  useEffect(() => {
    intentRef.current = onIntent;
  }, [onIntent]);

  useEffect(() => {
    connectionRef.current = onConnectionChange;
  }, [onConnectionChange]);

  // Presence, always live and free: the browser tells us when a pad appears or
  // goes away, so nothing has to spin to find out.
  useEffect(() => {
    const sync = () => {
      const count = connectedGamepads().length;
      setGamepadCount((current) => (current === count ? current : count));
    };
    sync();
    window.addEventListener('gamepadconnected', sync);
    window.addEventListener('gamepaddisconnected', sync);
    return () => {
      window.removeEventListener('gamepadconnected', sync);
      window.removeEventListener('gamepaddisconnected', sync);
    };
  }, []);

  const connected = gamepadCount > 0;
  const previousConnectedRef = useRef(false);
  useEffect(() => {
    if (previousConnectedRef.current === connected) return;
    previousConnectedRef.current = connected;
    connectionRef.current?.(connected);
  }, [connected]);

  // Input polling, which only runs while a pad is actually present. There is
  // nothing to read otherwise, so this needs no switch: a user who has never
  // touched a controller never starts an animation-frame loop, and one who
  // picks a controller up gets input without having turned anything on.
  useEffect(() => {
    if (!connected) return;

    let frame = 0;
    let previousButtons: boolean[] = [];
    let heldDirection: GamepadDirection | null = null;
    let nextRepeatAt = 0;

    const poll = (timestamp: number) => {
      const gamepads = connectedGamepads();
      const gamepad = gamepads[0];

      // Keep the count honest while polling. Some platforms expose a pad only
      // after the first input, which arrives without a connection event.
      setGamepadCount((current) => (current === gamepads.length ? current : gamepads.length));

      if (!gamepad) {
        previousButtons = [];
        heldDirection = null;
        nextRepeatAt = 0;
      } else {
        const intents = buttonIntents(gamepad, previousButtons);
        for (const intent of intents) intentRef.current?.(intent);

        const direction = directionFor(gamepad);
        if (direction !== heldDirection) {
          heldDirection = direction;
          if (direction) {
            intentRef.current?.({ type: 'direction', direction });
            nextRepeatAt = timestamp + Math.max(0, initialRepeatDelayMs);
          } else {
            nextRepeatAt = 0;
          }
        } else if (direction && timestamp >= nextRepeatAt) {
          intentRef.current?.({ type: 'direction', direction });
          nextRepeatAt = timestamp + Math.max(1, repeatIntervalMs);
        }
      }

      frame = requestAnimationFrame(poll);
    };

    frame = requestAnimationFrame(poll);
    return () => cancelAnimationFrame(frame);
  }, [connected, initialRepeatDelayMs, repeatIntervalMs]);

  return { connected, gamepadCount };
}
