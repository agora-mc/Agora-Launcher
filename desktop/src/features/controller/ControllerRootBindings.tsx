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
 * `menu` is deliberately not bound here. The handheld shell claims that button
 * to reopen itself, and it is registered above this layer; binding it here as
 * well would be dead code that looks live. When handheld mode stops being a
 * separate destination, Start becomes the palette and `context` can go back to
 * meaning nothing globally.
 */
export function ControllerRootBindings({
  rootRef,
  onOpenPalette,
  onCyclePage,
  onBack,
}: ControllerRootBindingsProps) {
  const onIntent = useCallback((intent: ControllerIntent) => {
    if (intent.type === 'context') {
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
