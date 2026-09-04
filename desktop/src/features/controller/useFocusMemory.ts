import { useEffect, useRef } from 'react';
import { GAMEPAD_MODALITY, INPUT_MODALITY_ATTRIBUTE } from './inputModality';

/**
 * Remember where focus was on each destination, and put it back on return.
 *
 * Without this, leaving a page and coming back drops focus to the top of the
 * document. With a mouse that is invisible — the pointer is wherever the user
 * left it. With a controller it means every return trip costs a long walk back
 * down the page, which is the difference between "navigable" and "usable".
 *
 * Deliberately gated on gamepad modality. Moving a mouse user's focus on a tab
 * switch is surprising and can steal it from something they are typing in; the
 * problem being solved here simply does not exist for them.
 */

/** Elements that must never be auto-focused on arrival. */
function isRestorable(element: Element | null): element is HTMLElement {
  if (!(element instanceof HTMLElement)) return false;
  if (!element.isConnected) return false;
  if (element === document.body) return false;
  if ('disabled' in element && Boolean((element as HTMLElement & { disabled?: boolean }).disabled)) {
    return false;
  }
  return true;
}

function usingGamepad(): boolean {
  if (typeof document === 'undefined') return false;
  return document.documentElement.getAttribute(INPUT_MODALITY_ATTRIBUTE) === GAMEPAD_MODALITY;
}

export function useFocusMemory(key: string): void {
  // One element per destination, so this is bounded by the number of
  // destinations rather than by how much the user navigates.
  const memory = useRef(new Map<string, HTMLElement>());
  const previousKey = useRef(key);

  useEffect(() => {
    const departed = previousKey.current;
    if (departed === key) return;
    previousKey.current = key;

    if (isRestorable(document.activeElement)) {
      memory.current.set(departed, document.activeElement as HTMLElement);
    } else {
      memory.current.delete(departed);
    }

    if (!usingGamepad()) return;

    // The arriving page has not necessarily rendered yet, and a remembered
    // element may have been unmounted while away, so this is a best effort
    // taken after layout rather than a guarantee.
    const frame = requestAnimationFrame(() => {
      const remembered = memory.current.get(key);
      if (remembered && isRestorable(remembered)) {
        remembered.focus({ preventScroll: true });
        // Not every environment implements this — jsdom has no layout at all,
        // and it is a progressive enhancement rather than the point: focus has
        // already moved by here.
        remembered.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
      } else {
        memory.current.delete(key);
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [key]);
}

/** Stable identity for a destination, used as the memory key. */
export function focusMemoryKey(destination: {
  type: string;
  tab?: string;
  itemId?: string;
  instanceId?: string;
}): string {
  if (destination.type === 'tab') return `tab:${destination.tab ?? ''}`;
  if (destination.type === 'mod-detail') return `mod:${destination.itemId ?? ''}`;
  if (destination.type === 'instance-detail') return `instance:${destination.instanceId ?? ''}`;
  return destination.type;
}
