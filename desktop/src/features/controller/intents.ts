/**
 * Semantic controller intents.
 *
 * Deliberately *not* button labels. `a`/`b` are Xbox names that mean different
 * things on a DualSense and are swapped outright on a Nintendo layout, so the
 * rest of the app must never see them. The physical mapping lives in the raw
 * sampling layer (`lib/useGamepad`); everything above this file reasons about
 * what the user meant.
 */

export type ControllerDirection = 'up' | 'down' | 'left' | 'right';

export type ControllerIntent =
  /** D-pad or left stick. Moves focus. */
  | { type: 'navigate'; direction: ControllerDirection }
  /** Activate the focused control. */
  | { type: 'accept' }
  /** Dismiss the topmost layer, or go back when nothing is stacked. */
  | { type: 'cancel' }
  /** Contextual secondary action. Layer-defined; no global meaning. */
  | { type: 'secondary' }
  /** Contextual tertiary action. Layer-defined; no global meaning. */
  | { type: 'context' }
  /** Open the command palette / app menu. */
  | { type: 'menu' }
  /** Shoulder buttons. Cycles tabs when no layer claims it. */
  | { type: 'page'; direction: 'prev' | 'next' }
  /** Right stick. Scrolls the nearest scrollport. */
  | { type: 'scroll'; direction: ControllerDirection };

/**
 * What a layer returns from `onIntent`.
 *
 * `true` stops dispatch. Anything else (including `undefined`, which is what a
 * handler with no explicit return gives back) falls through to the layer's
 * default navigation. Returning nothing must never silently swallow an intent —
 * that is how a controller appears to "stop working" on one screen.
 */
export type ControllerIntentResult = boolean | void;
