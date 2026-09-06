/**
 * Which device the user is currently driving the app with.
 *
 * This exists because `:focus-visible` cannot answer the question for us. The
 * browser decides that pseudo-class from its own input-modality heuristics,
 * which know about pointers and keyboards and nothing else. A gamepad is polled
 * rather than delivered as events, so moving focus with a controller never puts
 * the browser into keyboard modality — and a scripted `focus()` can instead
 * inherit whatever state the *previous* element had. The practical result is a
 * focus ring that appears or vanishes depending on what the user happened to
 * touch several interactions ago.
 *
 * So controller focus gets an explicit marker on the document element, and the
 * stylesheet keys the controller ring on that rather than guessing.
 */

export const INPUT_MODALITY_ATTRIBUTE = 'data-input-modality';

/** The value written while a controller is the active input device. */
export const GAMEPAD_MODALITY = 'gamepad';

function documentElement(): HTMLElement | null {
  if (typeof document === 'undefined') return null;
  return document.documentElement;
}

export function setGamepadModality(): void {
  documentElement()?.setAttribute(INPUT_MODALITY_ATTRIBUTE, GAMEPAD_MODALITY);
}

export function clearGamepadModality(): void {
  documentElement()?.removeAttribute(INPUT_MODALITY_ATTRIBUTE);
}

/**
 * Drop back to the browser's own focus heuristics the moment a mouse, wheel or
 * keyboard is used, so someone who puts the controller down does not keep a
 * controller-styled ring following their cursor around.
 *
 * Only trusted events count. Anything the app synthesises itself — a widget
 * adapter dispatching the arrow keys a Radix menu already understands, say —
 * is still the controller talking, and must not clear the marker.
 */
/**
 * The guard itself, separated from the wiring so it can be tested.
 *
 * jsdom defines `isTrusted` as a non-configurable own property, exactly as the
 * spec's `[LegacyUnforgeable]` requires, so a test cannot manufacture a trusted
 * event to dispatch. Calling this directly is the only way to cover the branch
 * that matters.
 */
export function handleDirectInput(event: Pick<Event, 'isTrusted'>): void {
  if (!event.isTrusted) return;
  clearGamepadModality();
}

export function watchForDirectInput(): () => void {
  if (typeof window === 'undefined') return () => undefined;

  window.addEventListener('pointerdown', handleDirectInput, true);
  window.addEventListener('keydown', handleDirectInput, true);
  window.addEventListener('wheel', handleDirectInput, { capture: true, passive: true });

  return () => {
    window.removeEventListener('pointerdown', handleDirectInput, true);
    window.removeEventListener('keydown', handleDirectInput, true);
    window.removeEventListener('wheel', handleDirectInput, true);
  };
}
