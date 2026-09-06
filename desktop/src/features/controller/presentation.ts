/**
 * Couch presentation: the same app, sized for a handheld screen or a television.
 *
 * This replaces what handheld mode used to be. That was a separate screen which
 * could list instances and launch one, and nothing else — a second set of
 * destinations that had to be kept in step with the real app, and predictably
 * was not. There is nothing to keep in step now, because there is no second set
 * of screens: this is one attribute on the document and some CSS.
 *
 * Driven by controller presence rather than a setting, which is the same
 * decision handheld mode was built on and worth keeping: picking the pad up is
 * the request. The Web Gamepad API only reports a pad once a button has been
 * pressed, so this follows a deliberate act rather than a device left plugged
 * in for something else.
 *
 * Note this deliberately tracks *presence*, not the input-modality marker in
 * `inputModality.ts`. Modality flips the moment a mouse is touched, which is
 * right for a focus ring and wrong for layout — resizing the whole interface
 * every time a hand moves between pad and mouse would be unusable.
 */

export const PRESENTATION_ATTRIBUTE = 'data-presentation';

/** The value written while the app is sized for controller use. */
export const COUCH_PRESENTATION = 'couch';

export function setCouchPresentation(active: boolean): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  if (active) root.setAttribute(PRESENTATION_ATTRIBUTE, COUCH_PRESENTATION);
  else root.removeAttribute(PRESENTATION_ATTRIBUTE);
}
