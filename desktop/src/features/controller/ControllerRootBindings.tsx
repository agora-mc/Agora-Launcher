import { useCallback, type RefObject } from 'react';
import type { ControllerIntent } from './intents';
import { CONTROLLER_LAYER_ROOT, useControllerLayer } from './useControllerLayer';

export interface ControllerRootBindingsProps {
  /** The application shell, whose focusable contents navigation walks. */
  rootRef: RefObject<HTMLElement | null>;
  /** Open the command palette — the "go anywhere" affordance. */
  onOpenPalette: () => void;
  /** Move one tab along the sidebar. */
  onCyclePage: (direction: 'prev' | 'next') => void;
  /** Leave the current destination. Must already refuse to exit the app. */
  onBack: () => void;
}

/**
 * The bottom of the layer stack: what a controller does when nothing more
 * specific has claimed the input.
 *
 * This is a component rather than a hook call inside `App` because `App`
 * early-returns for onboarding and the splash screen, and the values these
 * bindings need are computed after that point. A child component gets its own
 * unconditional hooks, and `CONTROLLER_LAYER_ROOT` keeps it underneath every
 * dialog regardless of the order effects happen to run in.
 *
 * Start opens the command palette, which is the "go anywhere" affordance: it
 * already searches destinations and instances, so it saves a controller user
 * from walking the sidebar to reach anything. It used to belong to the handheld
 * shell, which no longer exists as a separate destination.
 */
export function ControllerRootBindings({
  rootRef,
  onOpenPalette,
  onCyclePage,
  onBack,
}: ControllerRootBindingsProps) {
  const onIntent = useCallback((intent: ControllerIntent) => {
    if (intent.type === 'menu') {
      onOpenPalette();
      return true;
    }
    if (intent.type === 'page') {
      onCyclePage(intent.direction);
      return true;
    }
    return undefined;
  }, [onCyclePage, onOpenPalette]);

  useControllerLayer({
    rootRef,
    priority: CONTROLLER_LAYER_ROOT,
    onIntent,
    onCancel: onBack,
  });

  return null;
}

/** Step one place along a list of tabs, stopping at either end. */
export function cycleTab<T>(tabs: readonly T[], current: T, direction: 'prev' | 'next'): T {
  const index = tabs.indexOf(current);
  if (index < 0) return tabs[0] ?? current;
  // Clamped rather than wrapping: a shoulder button held down should come to
  // rest at the end of the list, not loop back past Home indefinitely.
  const next = direction === 'next'
    ? Math.min(index + 1, tabs.length - 1)
    : Math.max(index - 1, 0);
  return tabs[next] ?? current;
}
