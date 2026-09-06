import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from 'react';
import type { ControllerIntent, ControllerIntentResult } from './intents';
import {
  type ControllerLayerRegistration,
  useControllerLayerRegistry,
} from './ControllerProvider';

/**
 * Where a layer sits in the stack, independent of when it mounted.
 *
 * Registration order alone is not enough. React runs child effects before
 * parent effects, so the application shell — which must sit *underneath*
 * everything — would otherwise register last and end up on top of the very
 * dialogs it is supposed to yield to.
 */
export const CONTROLLER_LAYER_ROOT = -1;
export const CONTROLLER_LAYER_DEFAULT = 0;
/** Radix popovers: above a dialog, since a select opened inside one owns input. */
export const CONTROLLER_LAYER_POPOVER = 8;
export const CONTROLLER_LAYER_OVERLAY = 10;

export interface UseControllerLayerOptions {
  /** Register only while true. */
  active?: boolean;
  /** Container whose focusable descendants default navigation walks. */
  rootRef: RefObject<HTMLElement | null>;
  /** Return true to claim the intent; anything else falls through. */
  onIntent?: (intent: ControllerIntent) => ControllerIntentResult;
  /** Invoked for an unclaimed `cancel`. */
  onCancel?: () => void;
  /** Higher layers receive input first. Defaults to `CONTROLLER_LAYER_DEFAULT`. */
  priority?: number;
  /**
   * Let unclaimed intents continue to the layer below instead of stopping here.
   *
   * The default is opaque, because that is what a dialog needs: input must not
   * reach the page behind it. A transparent layer is for something that wants
   * one specific binding without otherwise owning input — a closed shell that
   * still listens for the button which reopens it, say. Marking such a layer
   * opaque silently disables the controller everywhere else in the app.
   */
  transparent?: boolean;
}

export function useControllerLayer({
  active = true,
  rootRef,
  onIntent,
  onCancel,
  priority = CONTROLLER_LAYER_DEFAULT,
  transparent = false,
}: UseControllerLayerOptions): void {
  const registry = useControllerLayerRegistry();
  const onIntentRef = useRef<UseControllerLayerOptions['onIntent']>(onIntent);
  const onCancelRef = useRef<UseControllerLayerOptions['onCancel']>(onCancel);
  onIntentRef.current = onIntent;
  onCancelRef.current = onCancel;

  const layer = useMemo<ControllerLayerRegistration>(
    () => ({ rootRef, onIntentRef, onCancelRef, priority, transparent }),
    [rootRef, onIntentRef, onCancelRef, priority, transparent],
  );

  useEffect(() => {
    if (!active || !registry) return undefined;
    return registry.registerLayer(layer);
  }, [active, layer, registry]);
}

/**
 * Controller ownership for a Radix popover — a select menu, a dropdown menu.
 *
 * These are the same trap the dialogs were. They render into a portal with
 * their own roving focus, they are *not* dialogs so the shared `DialogContent`
 * fix does not reach them, and without a layer the page behind them keeps
 * control while the popup itself cannot be driven.
 *
 * Presence comes from the ref callback rather than from mounting, because Radix
 * leaves these components mounted and returns null from the portal when closed
 * — so keying off mounting would leave every closed menu in the app claiming
 * input with a root ref pointing at nothing.
 *
 * Returns a ref callback to attach to the content element.
 */
export function useControllerPopoverLayer(): (node: HTMLElement | null) => void {
  const contentRef = useRef<HTMLElement | null>(null);
  const [present, setPresent] = useState(false);

  const register = useCallback((node: HTMLElement | null) => {
    contentRef.current = node;
    setPresent(node !== null);
  }, []);

  useControllerLayer({
    active: present,
    rootRef: contentRef,
    priority: CONTROLLER_LAYER_POPOVER,
    // Radix owns dismissal and listens for Escape on the document. Only
    // *default* actions are withheld from untrusted events, so a synthesised
    // key still reaches a listener.
    onCancel: useCallback(() => {
      contentRef.current?.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
      );
    }, []),
  });

  return register;
}
