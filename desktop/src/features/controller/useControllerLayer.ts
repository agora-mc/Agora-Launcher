import { useEffect, useMemo, useRef, type RefObject } from 'react';
import type { ControllerIntent, ControllerIntentResult } from './intents';
import {
  type ControllerLayerRegistration,
  useControllerLayerRegistry,
} from './ControllerProvider';

export interface UseControllerLayerOptions {
  active?: boolean;
  rootRef: RefObject<HTMLElement | null>;
  onIntent?: (intent: ControllerIntent) => ControllerIntentResult;
  onCancel?: () => void;
}

export function useControllerLayer({
  active = true,
  rootRef,
  onIntent,
  onCancel,
}: UseControllerLayerOptions): void {
  const registry = useControllerLayerRegistry();
  const onIntentRef = useRef<UseControllerLayerOptions['onIntent']>(onIntent);
  const onCancelRef = useRef<UseControllerLayerOptions['onCancel']>(onCancel);
  onIntentRef.current = onIntent;
  onCancelRef.current = onCancel;

  const layer = useMemo<ControllerLayerRegistration>(
    () => ({ rootRef, onIntentRef, onCancelRef }),
    [rootRef, onIntentRef, onCancelRef],
  );

  useEffect(() => {
    if (!active || !registry) return undefined;
    return registry.registerLayer(layer);
  }, [active, layer, registry]);
}
