/**
 * Guided walkthrough — public surface.
 *
 * The app shell mounts `TourProvider` and `TourOverlay`; entry points
 * (Settings, Help & Guide, the home dashboard) drop in `TourStartButton`.
 *
 * Two callers deliberately do NOT come through here. Pages that only *report*
 * to a running tour import `emitTourSignal` from `./tourSignals`, and the
 * onboarding wizard imports `queueTourStart` from `./tourHandoff`. Both are
 * plain functions with no React and no CSS behind them, and importing them
 * through this barrel would pull the overlay — stylesheet included — into
 * every page that installs a mod.
 */

export { TourProvider, useTour, type TourContextValue } from './TourProvider';
export { TourOverlay } from './TourOverlay';
export { TourStartButton } from './TourStartButton';
export { consumeQueuedTourStart } from './tourHandoff';
