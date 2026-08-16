/**
 * Operation signals for the guided walkthrough.
 *
 * Most tour steps advance on something the DOM can see (a page mounts, a
 * dialog opens, a button is clicked). Two cannot: "the instance was actually
 * created" and "the install actually finished" are indistinguishable from a
 * cancelled dialog by looking at the DOM alone. Those two places call
 * `emitTourSignal`.
 *
 * This is a one-function window event so instrumented components take on no
 * React context, no provider ordering, and no import of the tour UI — calling
 * it when no tour is running is a no-op.
 */

import type { TourSignal } from './tourModel';

export const TOUR_SIGNAL_EVENT = 'agora:tour-signal';

/** Report a completed operation to a running tour. Safe to call anywhere. */
export function emitTourSignal(signal: TourSignal): void {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent<TourSignal>(TOUR_SIGNAL_EVENT, { detail: signal }));
}

/** Subscribe to signals. Returns the unsubscribe function. */
export function subscribeTourSignals(handler: (signal: TourSignal) => void): () => void {
  if (typeof window === 'undefined') return () => undefined;
  const listener = (event: Event) => {
    const detail = (event as CustomEvent<TourSignal>).detail;
    if (detail) handler(detail);
  };
  window.addEventListener(TOUR_SIGNAL_EVENT, listener);
  return () => window.removeEventListener(TOUR_SIGNAL_EVENT, listener);
}
