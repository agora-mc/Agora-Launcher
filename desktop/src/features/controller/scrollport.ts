import type { ControllerDirection } from './intents';

const SCROLLABLE_OVERFLOW = new Set(['auto', 'overlay', 'scroll']);

function isScrollableOverflow(value: string): boolean {
  return SCROLLABLE_OVERFLOW.has(value);
}

function overflowForDirection(element: HTMLElement, direction: ControllerDirection): string {
  const style = window.getComputedStyle(element);
  const axisValue = direction === 'up' || direction === 'down' ? style.overflowY : style.overflowX;
  return axisValue || style.overflow;
}

function canScrollAxis(
  element: HTMLElement,
  direction: ControllerDirection,
  allowVisibleOverflow = false,
): boolean {
  const overflow = overflowForDirection(element, direction);
  if (!allowVisibleOverflow && !isScrollableOverflow(overflow)) return false;

  if (direction === 'up') return element.scrollTop > 0;
  if (direction === 'down') return element.scrollTop < element.scrollHeight - element.clientHeight;
  if (direction === 'left') return element.scrollLeft > 0;
  return element.scrollLeft < element.scrollWidth - element.clientWidth;
}

/**
 * The nearest ancestor that owns remaining scroll in the requested direction.
 * Checking remaining range here keeps a nested panel at its edge from
 * swallowing input that should continue to the larger panel around it.
 */
export function findScrollableAncestor(
  start: HTMLElement,
  direction: ControllerDirection,
): HTMLElement | null {
  let ancestor = start.parentElement;
  while (ancestor) {
    if (canScrollAxis(ancestor, direction)) return ancestor;
    ancestor = ancestor.parentElement;
  }

  const scrollingElement = document.scrollingElement;
  if (scrollingElement instanceof HTMLElement && canScrollAxis(scrollingElement, direction, true)) {
    return scrollingElement;
  }

  return null;
}

/** A viewport-sized nudge reveals new content without changing focus. */
export function scrollScrollableAncestor(
  scrollport: HTMLElement,
  direction: ControllerDirection,
): void {
  const distance = direction === 'up' || direction === 'down'
    ? Math.max(scrollport.clientHeight, 1)
    : Math.max(scrollport.clientWidth, 1);
  const amount = direction === 'up' || direction === 'left' ? -distance : distance;
  const options: ScrollToOptions = {
    behavior: 'auto',
    left: direction === 'left' || direction === 'right' ? amount : 0,
    top: direction === 'up' || direction === 'down' ? amount : 0,
  };

  if (typeof scrollport.scrollBy === 'function') {
    scrollport.scrollBy(options);
    return;
  }

  if (direction === 'up' || direction === 'down') {
    scrollport.scrollTop += amount;
  } else {
    scrollport.scrollLeft += amount;
  }
}

/** Keep scrolling layer-local by moving the port that contains the focused control. */
export function scrollNearestScrollport(
  start: HTMLElement,
  direction: ControllerDirection,
): HTMLElement | null {
  const scrollport = findScrollableAncestor(start, direction);
  if (!scrollport) return null;
  scrollScrollableAncestor(scrollport, direction);
  return scrollport;
}
