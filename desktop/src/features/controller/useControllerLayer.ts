import { useEffect, useMemo, useRef, type RefObject } from 'react';
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
