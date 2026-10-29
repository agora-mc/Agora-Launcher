/** Input types that take no typed text, so a global shortcut can safely fire from them. */
const NON_TEXT_INPUT_TYPES = new Set([
  'checkbox', 'radio', 'range', 'button', 'submit', 'reset', 'color', 'file', 'image',
]);

/**
 * True when typing into `target` produces text, so a global shortcut such as
 * Ctrl+K must leave the keystroke alone. A checkbox, radio or range slider is an
 * `<input>` too, but nothing typed there is lost by opening the palette.
 */
export function isTextEntryTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target instanceof HTMLTextAreaElement) return true;
  if (target instanceof HTMLInputElement) return !NON_TEXT_INPUT_TYPES.has(target.type);
  return target.isContentEditable || target.getAttribute('role') === 'textbox';
}
