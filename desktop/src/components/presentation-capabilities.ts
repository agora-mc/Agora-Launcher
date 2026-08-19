/**
 * Presentation capabilities — one capability object derived from the
 * interaction preference, not scattered `if (mode === …)` checks.
 *
 * Lives at the app boundary (not inside `features/interactive/`) because it
 * maps the interactive preference onto the AMBIENCE profile — and the
 * interactive layer must never import ambience. Both the AmbienceCoordinator
 * and any page-level surface read from this table.
 *
 * Note: eggs live in ambience, so Simple mode disabling eggs is expressed by
 * its ambience profile, not by a second switch inside the egg registry.
 *
 * Simple mode is deliberately the QUIET row: it keeps High Interaction's
 * structure for a single instance (hero play, icon shelf, pre-flight check,
 * crash doctor) but drops every decorative and gamified system, and it takes
 * Browse from the STANDARD page rather than the Bazaar. Its one cross-cutting
 * effect is `reducedMotion`, which the presentation-motion coordinator applies
 * to the app-wide motion preference — and reduced motion in turn switches the
 * living background off (see `AmbienceProvider`).
 */

import type { AmbienceProfile } from '../features/ambience/engine/engine';
import type { MotionPreference } from './theme/theme-provider';
import type { InteractionPreference } from '../features/interactive/live/presentationPreference';

/** Which discovery surface a mode browses with. */
export type BrowseSurface = 'standard' | 'simple' | 'bazaar';

export interface PresentationCapabilities {
  /** Icon shelf of tiles instead of a card list. */
  shelf: boolean;
  /** Bounded neighbourhood diagram on selection. */
  diagram: boolean;
  /** The ambience profile this mode defaults to. */
  ambience: AmbienceProfile;
  /** Decorative motion: rarity tiers, tilt, drag-reorder, scan wave, buddy, particles. */
  flourish: boolean;
  /** Easter eggs + Field Journal (driven by the ambience profile). */
  eggs: boolean;
  /** Ambient music. */
  music: boolean;
  /** The Browse surface this mode uses. */
  browse: BrowseSurface;
  /**
   * The mode pins the app-wide motion preference to `reduced`. Only Simple
   * does; Standard and High Interaction leave motion entirely to the user.
   */
  reducedMotion: boolean;
}

export const PRESENTATION_CAPS: Record<InteractionPreference, PresentationCapabilities> = {
  standard: { shelf: false, diagram: false, ambience: 'off', flourish: false, eggs: false, music: false, browse: 'standard', reducedMotion: false },
  simple: { shelf: true, diagram: false, ambience: 'off', flourish: false, eggs: false, music: false, browse: 'simple', reducedMotion: true },
  'high-interaction': { shelf: true, diagram: true, ambience: 'full', flourish: true, eggs: true, music: true, browse: 'bazaar', reducedMotion: false },
} as const;

export function presentationCapabilities(pref: InteractionPreference): PresentationCapabilities {
  return PRESENTATION_CAPS[pref] ?? PRESENTATION_CAPS.standard;
}

/**
 * The motion preference a mode pins, or `null` when the mode leaves the
 * setting to the user. Kept beside the caps table so "Simple means reduced
 * motion" is stated once.
 */
export function pinnedMotion(pref: InteractionPreference): MotionPreference | null {
  return presentationCapabilities(pref).reducedMotion ? 'reduced' : null;
}
