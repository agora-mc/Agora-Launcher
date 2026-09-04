import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MutableRefObject,
  type PropsWithChildren,
  type RefObject,
} from 'react';
import { useGamepad, type GamepadIntent } from '../../lib/useGamepad';
import { setGamepadModality, watchForDirectInput } from './inputModality';
import { setCouchPresentation } from './presentation';
import type { ControllerIntent, ControllerIntentResult } from './intents';
import { hasUsableGeometry, chooseCandidate, type NavRect } from './spatialNavigation';
import { scrollNearestScrollport } from './scrollport';
import { adaptAccept, adaptNavigate } from './elementAdapters';
import { isEditableField, type EditableField } from './textEditing';
import { OnScreenKeyboard } from './OnScreenKeyboard';
import { SelectOverlay } from './SelectOverlay';
import { ColorOverlay } from './ColorOverlay';

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

/**
 * Items of a composite widget that manages its own tab stop.
 *
 * A tablist, listbox or radiogroup keeps exactly one child at `tabIndex` 0 and
 * parks the rest at -1. For the keyboard that is correct — Tab enters the group
 * once and the arrows move within it. Controller navigation has no separate
 * "enter the group" step, so those parked children have to stay candidates or
 * the group becomes a dead end.
 */
const ROVING_ITEM_ROLES = new Set([
  'tab', 'option', 'radio', 'menuitem', 'menuitemcheckbox', 'menuitemradio',
  'treeitem', 'gridcell',
]);

function isRovingItem(element: HTMLElement): boolean {
  const role = element.getAttribute('role');
  return role !== null && ROVING_ITEM_ROLES.has(role);
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
    // Anything that opts out explicitly. The sidebar's resize grip is keyboard-
    // operable but has no controller meaning, and it sits between the sidebar
    // and the content where a stick runs straight into it.
    if (element.closest('[data-controller-skip]')) return false;
    // `tabindex="-1"` normally means "focusable by script, not part of the
    // sequence" — except in a composite widget, where roving tabindex parks -1
    // on every *unselected* item. Excluding those outright made the Settings
    // section tabs unreachable: only the one you were already on was a
    // candidate, so there was no way to move off it.
    if (element.getAttribute('tabindex') === '-1' && !isRovingItem(element)) return false;
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
  const active = document.activeElement;

  // A focused native control gets first refusal along its own axis. Otherwise
  // navigation would move focus off a select rather than changing it, and the
  // only other way to operate one is the OS popup a controller cannot reach.
  if (adaptNavigate(active, direction)) return;

  const elements = focusableElements(root);
  if (elements.length === 0) return;

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

function defaultAccept(
  root: HTMLElement,
  onOpenKeyboard: (field: EditableField) => void,
  onOpenSelect: (select: HTMLSelectElement) => void,
  onOpenColor: (input: HTMLInputElement) => void,
) {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement) || !root.contains(active)) return;
  if (!focusableElements(root).includes(active)) return;
  if (isEditableField(active)) {
    onOpenKeyboard(active);
    return;
  }
  // A native dropdown would open a system popup here. Hand its options to an
  // in-app overlay the controller can actually steer instead.
  if (active instanceof HTMLSelectElement) {
    onOpenSelect(active);
    return;
  }
  // Same reasoning: a colour input would otherwise open an OS colour dialog.
  if (active instanceof HTMLInputElement && active.type === 'color') {
    onOpenColor(active);
    return;
  }
  // Clicking a select or a colour swatch opens the operating-system widget the
  // adapters exist to keep the user out of, so those absorb Accept instead.
  if (adaptAccept(active)) return;
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
function dispatchDefault(
  root: HTMLElement | null,
  intent: ControllerIntent,
  onCancel: (() => void) | undefined,
  onOpenKeyboard: (field: EditableField) => void,
  onOpenSelect: (select: HTMLSelectElement) => void,
  onOpenColor: (input: HTMLInputElement) => void,
) {
  if (intent.type === 'cancel') {
    onCancel?.();
  } else if (root && intent.type === 'navigate') {
    defaultNavigate(root, intent.direction);
  } else if (root && intent.type === 'accept') {
    defaultAccept(root, onOpenKeyboard, onOpenSelect, onOpenColor);
  } else if (root && intent.type === 'scroll') {
    defaultScroll(root, intent.direction);
  }
}


/**
 * Attribute a hand-rolled dialog sets to claim controller input.
 *
 * Not every modal in this app is a Radix dialog. The High Interaction surfaces
 * build their own — a scrim div with `role="dialog"` inside it — and they live
 * behind an enforced import boundary that (correctly) forbids them from
 * importing app-level modules like this one. So instead of an import, they set
 * an attribute, and the provider picks them up.
 *
 * Set it only while the dialog is actually open. These scrims stay mounted and
 * hide themselves with opacity, so presence in the DOM is not the same question
 * as being on screen.
 *
 * Cancel dispatches a click on the element itself, which is the dismiss gesture
 * these scrims already implement for clicking outside the dialog body.
 */
export const CONTROLLER_DIALOG_ATTRIBUTE = 'data-controller-dialog';

/** Priority for an attribute-declared dialog: above ordinary layers, below the
 *  keyboard and select overlays which must sit on top of everything. */
const IMPLICIT_DIALOG_PRIORITY = 5;

function implicitDialogLayer(): ControllerLayerRegistration | null {
  if (typeof document === 'undefined') return null;
  const dialogs = document.querySelectorAll<HTMLElement>(`[${CONTROLLER_DIALOG_ATTRIBUTE}]`);
  const element = dialogs[dialogs.length - 1];
  if (!element) return null;

  return {
    rootRef: { current: element },
    onIntentRef: { current: undefined },
    onCancelRef: {
      current: () => {
        element.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
      },
    },
    priority: IMPLICIT_DIALOG_PRIORITY,
    transparent: false,
  };
}

export function ControllerProvider({ children }: PropsWithChildren) {
  const layersRef = useRef<ControllerLayerRegistration[]>([]);
  const keyboardFieldRef = useRef<EditableField | null>(null);
  const [keyboardField, setKeyboardField] = useState<EditableField | null>(null);
  const selectRef = useRef<HTMLSelectElement | null>(null);
  const [openSelectElement, setOpenSelectElement] = useState<HTMLSelectElement | null>(null);
  const colorRef = useRef<HTMLInputElement | null>(null);
  const [openColorElement, setOpenColorElement] = useState<HTMLInputElement | null>(null);

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

  const openKeyboard = useCallback((field: EditableField) => {
    keyboardFieldRef.current = field;
    setKeyboardField(field);
  }, []);

  const openSelect = useCallback((select: HTMLSelectElement) => {
    selectRef.current = select;
    setOpenSelectElement(select);
  }, []);

  const closeSelect = useCallback(() => {
    const select = selectRef.current;
    selectRef.current = null;
    setOpenSelectElement(null);
    if (select?.isConnected) select.focus({ preventScroll: true });
  }, []);

  const openColor = useCallback((input: HTMLInputElement) => {
    colorRef.current = input;
    setOpenColorElement(input);
  }, []);

  const closeColor = useCallback(() => {
    const input = colorRef.current;
    colorRef.current = null;
    setOpenColorElement(null);
    if (input?.isConnected) input.focus({ preventScroll: true });
  }, []);

  const closeKeyboard = useCallback(() => {
    const field = keyboardFieldRef.current;
    keyboardFieldRef.current = null;
    setKeyboardField(null);
    if (field?.isConnected) field.focus({ preventScroll: true });
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
    const implicit = implicitDialogLayer();
    const candidates = implicit ? [...layersRef.current, implicit] : layersRef.current;
    const ordered = [...candidates].sort((a, b) => a.priority - b.priority);

    for (let index = ordered.length - 1; index >= 0; index -= 1) {
      const layer = ordered[index];
      if (layer.onIntentRef.current?.(intent) === true) return;
      // An opaque layer stops here even when it did nothing with the intent:
      // that is what keeps input off the page behind a dialog. A transparent
      // one wanted a specific binding and nothing more, so keep walking down.
      if (!layer.transparent) {
        dispatchDefault(layer.rootRef.current, intent, layer.onCancelRef.current, openKeyboard, openSelect, openColor);
        return;
      }
    }
  }, [openKeyboard, openSelect, openColor]);

  const { connected, gamepadCount } = useGamepad({ onIntent: dispatch });
  // Size the whole app for couch distance while a pad is in play, and put it
  // back when the pad goes away.
  useEffect(() => {
    setCouchPresentation(connected);
    return () => setCouchPresentation(false);
  }, [connected]);
  const registry = useMemo(() => ({ registerLayer }), [registerLayer]);
  const state = useMemo(() => ({ connected, gamepadCount }), [connected, gamepadCount]);

  return (
    <ControllerStateContext.Provider value={state}>
      <ControllerLayerContext.Provider value={registry}>
        {children}
        {keyboardField && <OnScreenKeyboard field={keyboardField} onClose={closeKeyboard} />}
        {openSelectElement && <SelectOverlay select={openSelectElement} onClose={closeSelect} />}
        {openColorElement && <ColorOverlay input={openColorElement} onClose={closeColor} />}
      </ControllerLayerContext.Provider>
    </ControllerStateContext.Provider>
  );
}
