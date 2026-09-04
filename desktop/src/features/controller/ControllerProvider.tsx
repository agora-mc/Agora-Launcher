import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  type MutableRefObject,
  type PropsWithChildren,
  type RefObject,
} from 'react';
import { useGamepad, type GamepadIntent } from '../../lib/useGamepad';
import { setGamepadModality, watchForDirectInput } from './inputModality';
import type { ControllerIntent, ControllerIntentResult } from './intents';
import { hasUsableGeometry, chooseCandidate, type NavRect } from './spatialNavigation';
import { scrollNearestScrollport } from './scrollport';

export interface ControllerLayerRegistration {
  rootRef: RefObject<HTMLElement | null>;
  onIntentRef: MutableRefObject<((intent: ControllerIntent) => ControllerIntentResult) | undefined>;
  onCancelRef: MutableRefObject<(() => void) | undefined>;
  priority: number;
  transparent: boolean;
}

interface ControllerLayerRegistry {
  registerLayer: (layer: ControllerLayerRegistration) => () => void;
}

interface ControllerState {
  connected: boolean;
  gamepadCount: number;
}

const ControllerLayerContext = createContext<ControllerLayerRegistry | null>(null);
const ControllerStateContext = createContext<ControllerState>({ connected: false, gamepadCount: 0 });

export function useController(): ControllerState {
  return useContext(ControllerStateContext);
}

export function useControllerLayerRegistry(): ControllerLayerRegistry | null {
  return useContext(ControllerLayerContext);
}

function toControllerIntent(intent: GamepadIntent): ControllerIntent {
  if (intent.type === 'direction') return { type: 'navigate', direction: intent.direction };
  if (intent.type === 'stick') return { type: 'scroll', direction: intent.direction };

  switch (intent.button) {
    case 'south': return { type: 'accept' };
    case 'east': return { type: 'cancel' };
    case 'west': return { type: 'secondary' };
    case 'north': return { type: 'context' };
    case 'start': return { type: 'menu' };
    case 'l1': return { type: 'page', direction: 'prev' };
    case 'r1': return { type: 'page', direction: 'next' };
  }
}

function focusableElements(root: HTMLElement): HTMLElement[] {
  const candidates = root.querySelectorAll<HTMLElement>(
    'button, a[href], input, select, textarea, [tabindex], [contenteditable="true"]',
  );
  return Array.from(candidates).filter((element) => {
    if (element.hidden || element.getAttribute('aria-hidden') === 'true') return false;
    if (element.closest('[hidden], [inert], [aria-hidden="true"]')) return false;
    if ('disabled' in element && Boolean((element as HTMLElement & { disabled?: boolean }).disabled)) return false;
    if (element.getAttribute('aria-disabled') === 'true') return false;
    // `tabindex="-1"` means "focusable by script, not part of the sequence".
    // Roving-tabindex widgets park -1 on every unselected item, so including
    // them would make one logical control into N navigation stops.
    if (element.getAttribute('tabindex') === '-1') return false;
    // Conditional rendering and collapsed panels leave real elements in the
    // tree that no pointer could ever reach. jsdom has no layout and no
    // `checkVisibility`, so this is a browser-only refinement by design.
    if (typeof element.checkVisibility === 'function' && !element.checkVisibility()) return false;
    return true;
  });
}

function readRect(element: HTMLElement): NavRect {
  const rect = element.getBoundingClientRect();
  return {
    left: rect.left,
    top: rect.top,
    right: rect.right,
    bottom: rect.bottom,
  };
}

function focusControllerTarget(element: HTMLElement): void {
  element.focus({ preventScroll: true });
  element.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
}

function documentOrderTarget(
  elements: HTMLElement[],
  current: number,
  direction: Extract<ControllerIntent, { type: 'navigate' }>['direction'],
): HTMLElement | undefined {
  const step = direction === 'up' || direction === 'left' ? -1 : 1;
  const next = current < 0
    ? (step < 0 ? elements.length - 1 : 0)
    : (current + step + elements.length) % elements.length;
  return elements[next];
}

function defaultNavigate(
  root: HTMLElement,
  direction: Extract<ControllerIntent, { type: 'navigate' }>['direction'],
) {
  const elements = focusableElements(root);
  if (elements.length === 0) return;

  const active = document.activeElement;
  const current = active instanceof HTMLElement ? elements.indexOf(active) : -1;
  const nextInOrder = () => {
    const target = documentOrderTarget(elements, current, direction);
    if (target) focusControllerTarget(target);
  };

  // There is no meaningful origin before the layer has focus. Preserve the
  // initial document-order entry, which also handles focus in another layer.
  if (current < 0) {
    nextInOrder();
    return;
  }

  const rects = elements.map(readRect);
  const origin = rects[current];
  if (!origin || !hasUsableGeometry(rects) || !hasUsableGeometry([origin])) {
    nextInOrder();
    return;
  }

  const candidates = elements
    .map((item, index) => ({ item, rect: rects[index] }))
    .filter(({ item }) => item !== active);
  let target = chooseCandidate(origin, candidates, direction);
  if (target) {
    focusControllerTarget(target);
    return;
  }

  if (!(active instanceof HTMLElement)) return;
  if (!scrollNearestScrollport(active, direction)) return;

  // Scrolling changes viewport-relative rectangles, so every retry gets a
  // fresh snapshot instead of using geometry from before the scroll.
  const retryRects = elements.map(readRect);
  const retryOrigin = retryRects[current];
  if (!retryOrigin || !hasUsableGeometry(retryRects) || !hasUsableGeometry([retryOrigin])) return;
  target = chooseCandidate(
    retryOrigin,
    elements
      .map((item, index) => ({ item, rect: retryRects[index] }))
      .filter(({ item }) => item !== active),
    direction,
  );
  if (target) focusControllerTarget(target);
}

function defaultAccept(root: HTMLElement) {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement) || !root.contains(active)) return;
  if (!focusableElements(root).includes(active)) return;
  active.click();
}

function defaultScroll(root: HTMLElement, direction: Extract<ControllerIntent, { type: 'scroll' }>['direction']) {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement) || !root.contains(active)) return;
  scrollNearestScrollport(active, direction);
}

/**
 * What an owning layer gets for free when it does not claim an intent.
 *
 * `menu` and `page` are deliberately absent: they have app-level meanings.
 * Navigation and right-stick scrolling stay layer-local because their target
 * is the focused element and its owning scrollport.
 */
function dispatchDefault(root: HTMLElement | null, intent: ControllerIntent, onCancel?: () => void) {
  if (intent.type === 'cancel') {
    onCancel?.();
  } else if (root && intent.type === 'navigate') {
    defaultNavigate(root, intent.direction);
  } else if (root && intent.type === 'accept') {
    defaultAccept(root);
  } else if (root && intent.type === 'scroll') {
    defaultScroll(root, intent.direction);
  }
}

export function ControllerProvider({ children }: PropsWithChildren) {
  const layersRef = useRef<ControllerLayerRegistration[]>([]);

  const registerLayer = useCallback((layer: ControllerLayerRegistration) => {
    layersRef.current.push(layer);
    let registered = true;
    return () => {
      if (!registered) return;
      registered = false;
      const index = layersRef.current.indexOf(layer);
      if (index >= 0) layersRef.current.splice(index, 1);
    };
  }, []);

  useEffect(() => watchForDirectInput(), []);

  const dispatch = useCallback((rawIntent: GamepadIntent) => {
    // Mark modality before anything else, including when no layer claims the
    // intent: the user is demonstrably on a controller either way, and the ring
    // has to be right on the very first press rather than the second.
    setGamepadModality();

    const intent = toControllerIntent(rawIntent);

    // Highest priority first, and within one priority the most recently
    // registered first. `sort` is stable, so registration order survives.
    const ordered = [...layersRef.current].sort((a, b) => a.priority - b.priority);

    for (let index = ordered.length - 1; index >= 0; index -= 1) {
      const layer = ordered[index];
      if (layer.onIntentRef.current?.(intent) === true) return;
      // An opaque layer stops here even when it did nothing with the intent:
      // that is what keeps input off the page behind a dialog. A transparent
      // one wanted a specific binding and nothing more, so keep walking down.
      if (!layer.transparent) {
        dispatchDefault(layer.rootRef.current, intent, layer.onCancelRef.current);
        return;
      }
    }
  }, []);

  const { connected, gamepadCount } = useGamepad({ onIntent: dispatch });
  const registry = useMemo(() => ({ registerLayer }), [registerLayer]);
  const state = useMemo(() => ({ connected, gamepadCount }), [connected, gamepadCount]);

  return (
    <ControllerStateContext.Provider value={state}>
      <ControllerLayerContext.Provider value={registry}>
        {children}
      </ControllerLayerContext.Provider>
    </ControllerStateContext.Provider>
  );
}
