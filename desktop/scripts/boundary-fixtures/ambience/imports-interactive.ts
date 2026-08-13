/**
 * NEGATIVE FIXTURE — must violate the ambience boundary.
 * `features/ambience/` may never import the interactive layer.
 */

import { loadPreference } from '../interactive/live/presentationPreference';

export function mode(): string {
  return loadPreference();
}
