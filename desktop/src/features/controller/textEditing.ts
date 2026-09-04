/**
 * Caret-aware text editing for controller-driven input.
 *
 * The naive on-screen keyboard appends to the end of the value and calls it
 * done. That is fine until someone moves the caret, or the field already had
 * text in it, at which point every keystroke lands in the wrong place. These
 * functions edit at the selection instead, the way a real keypress would, and
 * leave the caret where the user would expect to find it.
 *
 * They are deliberately narrow. This is a basic-Latin fallback for entering a
 * search term or a folder name with a gamepad — not an input method. It does
 * not do composition, dead keys, or bidirectional text, and it should not grow
 * to: a platform text-input adapter is the right answer for those, and this
 * stays small enough to be obviously correct in the meantime.
 */
import { setNativeValue } from './elementAdapters';

export type EditableField = HTMLInputElement | HTMLTextAreaElement;

/** Fields this keyboard is willing to edit. */
const TEXTUAL_INPUT_TYPES = new Set(['text', 'search', 'url', 'email', 'tel', 'password', '']);

export function isEditableField(element: Element | null): element is EditableField {
  if (element instanceof HTMLTextAreaElement) return !element.readOnly && !element.disabled;
  if (!(element instanceof HTMLInputElement)) return false;
  if (element.readOnly || element.disabled) return false;
  return TEXTUAL_INPUT_TYPES.has(element.type);
}

function prototypeFor(field: EditableField): object {
  return field instanceof HTMLTextAreaElement
    ? HTMLTextAreaElement.prototype
    : HTMLInputElement.prototype;
}

/**
 * Where the caret is, defaulting to the end of the value.
 *
 * `selectionStart` is null on input types that do not support selection, and
 * reading it can throw on some of them, so this never assumes it is available.
 */
function selection(field: EditableField): { start: number; end: number } {
  const length = field.value.length;
  try {
    const start = field.selectionStart;
    const end = field.selectionEnd;
    if (start === null || end === null) return { start: length, end: length };
    return { start: Math.min(start, end), end: Math.max(start, end) };
  } catch {
    return { start: length, end: length };
  }
}

function setCaret(field: EditableField, position: number): void {
  try {
    field.setSelectionRange?.(position, position);
  } catch {
    // Not every input type supports a selection range; the value is already
    // correct by this point and the caret is cosmetic.
  }
}

function apply(field: EditableField, value: string, caret: number): boolean {
  const written = setNativeValue(field, prototypeFor(field), value, ['input', 'change']);
  if (!written) return false;
  setCaret(field, caret);
  return true;
}

/** Insert text at the caret, replacing any selected range. */
export function insertText(field: EditableField, text: string): boolean {
  if (!text) return false;
  const { start, end } = selection(field);
  const value = field.value;

  if (field.maxLength >= 0) {
    const room = field.maxLength - (value.length - (end - start));
    if (room <= 0) return false;
    text = text.slice(0, room);
    if (!text) return false;
  }

  return apply(field, value.slice(0, start) + text + value.slice(end), start + text.length);
}

/**
 * Delete backwards from the caret, or delete the selection when there is one.
 *
 * Deleting by code point rather than by code unit, so one press removes one
 * emoji instead of half of a surrogate pair and leaving a broken character
 * behind.
 */
export function deleteBackwards(field: EditableField): boolean {
  const { start, end } = selection(field);
  const value = field.value;

  if (start !== end) return apply(field, value.slice(0, start) + value.slice(end), start);
  if (start === 0) return false;

  const previous = Array.from(value.slice(0, start)).pop() ?? '';
  const removed = previous.length || 1;
  return apply(field, value.slice(0, start - removed) + value.slice(start), start - removed);
}

/** Move the caret one code point, without changing the value. */
export function moveCaret(field: EditableField, direction: 'left' | 'right'): boolean {
  const { start, end } = selection(field);
  const value = field.value;

  if (start !== end) {
    setCaret(field, direction === 'left' ? start : end);
    return true;
  }

  if (direction === 'left') {
    if (start === 0) return false;
    const previous = Array.from(value.slice(0, start)).pop() ?? '';
    setCaret(field, start - (previous.length || 1));
    return true;
  }

  if (start >= value.length) return false;
  const next = Array.from(value.slice(start))[0] ?? '';
  setCaret(field, start + (next.length || 1));
  return true;
}

/** Empty the field, as a "clear" key would. */
export function clearField(field: EditableField): boolean {
  if (field.value === '') return false;
  return apply(field, '', 0);
}
