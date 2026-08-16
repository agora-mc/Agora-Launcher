/**
 * DOM lookups for tour anchors.
 *
 * The only contract between the tour and the rest of the app is the
 * `data-tour="<anchor>"` attribute, so everything the overlay needs to know
 * about the live UI goes through the four helpers here.
 */

import type { TourAnchor } from './tourModel';

/** Anchors are kebab-case identifiers, so plain attribute selectors are safe. */
export const ANCHOR_PATTERN = /^[a-z0-9]+(-[a-z0-9]+)*$/;

export function anchorSelector(anchor: TourAnchor): string {
  return `[data-tour="${anchor}"]`;
}

/**
 * Whether an element counts as on screen.
 *
 * Presence alone is not enough: the app keeps Browse and the instance editor
 * mounted-but-`display:none` behind a mod detail page, and a step must not
 * treat a hidden page as the page the user is looking at. `checkVisibility()`
 * answers that in one call on the engines Tauri ships; where it is missing
 * (older WebKit, jsdom in unit tests) we fall back to plain presence, which
 * only makes the tour slightly more eager.
 */
export function isAnchorVisible(element: Element): boolean {
  if (!element.isConnected) return false;
  const check = (element as Element & { checkVisibility?: () => boolean }).checkVisibility;
  if (typeof check === 'function') return check.call(element);
  return true;
}

/** The first visible element declaring `anchor`, or null. */
export function findAnchor(anchor: TourAnchor): HTMLElement | null {
  if (typeof document === 'undefined') return null;
  const matches = document.querySelectorAll<HTMLElement>(anchorSelector(anchor));
  for (const element of matches) {
    if (isAnchorVisible(element)) return element;
  }
  return null;
}

export function isAnchorPresent(anchor: TourAnchor): boolean {
  return findAnchor(anchor) !== null;
}

/**
 * The app's motion rule, matching `index.css` and `tour.css`: the appearance
 * setting wins outright, and the OS preference applies unless the user has
 * explicitly asked for full motion.
 */
export function prefersReducedMotion(): boolean {
  if (typeof document === 'undefined') return false;
  const setting = document.documentElement.getAttribute('data-motion');
  if (setting === 'reduced') return true;
  if (setting === 'full') return false;
  return typeof window !== 'undefined'
    && typeof window.matchMedia === 'function'
    && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/**
 * Every anchor enclosing `node`, innermost first — an event target can sit
 * inside several nested anchors (a version row inside the version list).
 */
export function anchorsFromNode(node: EventTarget | null): TourAnchor[] {
  const found: TourAnchor[] = [];
  let element: Element | null = node instanceof Element ? node : null;
  while (element) {
    const value = element.getAttribute('data-tour');
    if (value) found.push(value);
    element = element.parentElement;
  }
  return found;
}
