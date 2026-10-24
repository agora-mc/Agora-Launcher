/**
 * Where keyboard focus lands when the page changes, and where "Skip to
 * content" goes.
 *
 * Without this, a page change leaves focus on a sidebar button or on an
 * element that has just been unmounted, so the next Tab starts from the top of
 * the window. Moving focus to the new page's heading means the next Tab starts
 * inside the page instead.
 */

import { GAMEPAD_MODALITY, INPUT_MODALITY_ATTRIBUTE } from '../features/controller/inputModality';

/**
 * The first visible heading in the content area, or the content area itself
 * when there is none. Pages title themselves with either an h1 or an h2, so
 * both count, in document order. Headings get `tabindex="-1"` so they can take
 * focus without joining the Tab order.
 */
export function routeFocusTarget(main: HTMLElement): HTMLElement {
  const heading = Array.from(main.querySelectorAll<HTMLElement>('h1, h2'))
    .find((h) => h.closest('[hidden], .hidden') === null);
  if (!heading) return main;
  if (!heading.hasAttribute('tabindex')) heading.tabIndex = -1;
  return heading;
}

/**
 * Whether a page change should move focus. Focus that is on the sidebar, in
 * the content, or lost entirely (on the body) follows the page. Focus inside a
 * dialog or anything else outside the shell is the user's own, and stays put.
 */
export function focusFollowsRoute(active: Element | null, main: Element, sidebar: Element | null): boolean {
  if (!active || active === active.ownerDocument.body) return true;
  if (main.contains(active)) return true;
  return sidebar !== null && sidebar.contains(active);
}

/**
 * The page-change focus move, for keyboard and mouse users. Controller users
 * are left to focus memory, which puts them back where they were on the page;
 * both moving focus would fight over the same element.
 */
export function moveFocusForRoute(main: HTMLElement, sidebar: Element | null): void {
  if (document.documentElement.getAttribute(INPUT_MODALITY_ATTRIBUTE) === GAMEPAD_MODALITY) return;
  if (!focusFollowsRoute(document.activeElement, main, sidebar)) return;
  routeFocusTarget(main).focus({ preventScroll: true });
}
