/**
 * Presentation capabilities — one capability object derived from the
 * interaction preference, not scattered `if (mode === …)` checks
 * (V5-PORT-PLAN §10).
 *
 * Lives at the app boundary (not inside `features/interactive/`) because it
 * maps the interactive preference onto the AMBIENCE profile — and the
 * interactive layer must never import ambience (V5-PORT-PLAN §3). Both the
 * AmbienceCoordinator and any page-level surface read from this table.
 *
 * Note: eggs live in ambience, so Simple mode disabling eggs is expressed by
 * its ambience profile, not by a second switch inside the egg registry.
 */

import type { AmbienceProfile } from '../features/ambience/engine/engine';
import type { InteractionPreference } from '../features/interactive/live/presentationPreference';

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
}

export const PRESENTATION_CAPS: Record<InteractionPreference, PresentationCapabilities> = {
  standard: { shelf: false, diagram: false, ambience: 'off', flourish: false, eggs: false, music: false },
  simple: { shelf: true, diagram: true, ambience: 'off', flourish: false, eggs: false, music: false },
  'high-interaction': { shelf: true, diagram: true, ambience: 'full', flourish: true, eggs: true, music: true },
} as const;

export function presentationCapabilities(pref: InteractionPreference): PresentationCapabilities {
  return PRESENTATION_CAPS[pref] ?? PRESENTATION_CAPS.standard;
}
