/**
 * Making native form controls usable from a controller.
 *
 * A `<select>`, a range slider and a colour swatch all open operating-system
 * widgets when activated. Those are drawn outside the page, the Gamepad API
 * cannot see them, and no amount of focus management reaches inside one. That
 * is the real gap between "a controller can reach every control" and "a
 * controller can use the app": this codebase has 39 native selects and 13
 * sliders, most of them in Settings and the instance editor.
 *
 * The fix is to never open the OS widget at all. A focused select cycles its
 * options in place, a slider steps by its own increment, and neither ever hands
 * off to a popup the controller would be locked out of. That also avoids
 * rewriting eleven files of markup to a custom listbox, which would have been
 * the same behaviour with far more surface area.
 *
 * React is the wrinkle. It installs its own value tracker on form elements and
 * skips `onChange` when a value changes without going through it, so assigning
 * `element.value` directly updates the DOM and silently fails to update the
 * component. Every write here goes through the prototype's native setter, which
 * is what defeats that tracker.
 */
import type { ControllerDirection } from './intents';

type ValueSetter = ((value: string) => void) | undefined;

function nativeValueSetter(element: HTMLElement, prototype: object): ValueSetter {
  const descriptor = Object.getOwnPropertyDescriptor(prototype, 'value');
  const setter = descriptor?.set;
  if (!setter) return undefined;
  return (value: string) => setter.call(element, value);
}

/** Write a value the way a user would, so React's tracker notices. */
function commit(element: HTMLElement, prototype: object, value: string, events: string[]): boolean {
  const setter = nativeValueSetter(element, prototype);
  if (!setter) return false;
  setter(value);
  for (const type of events) {
    element.dispatchEvent(new Event(type, { bubbles: true }));
  }
  return true;
}

/** Which way a direction moves a one-dimensional control. */
function stepFor(direction: ControllerDirection): -1 | 1 {
  return direction === 'up' || direction === 'left' ? -1 : 1;
}

function selectableOptions(select: HTMLSelectElement): HTMLOptionElement[] {
  return Array.from(select.options).filter((option) => !option.disabled);
}

function adaptSelect(select: HTMLSelectElement, direction: ControllerDirection): boolean {
  // A multi-select is a list, not a single value; leave it to normal focus
  // movement rather than inventing a selection model for it.
  if (select.multiple) return false;

  const options = selectableOptions(select);
  if (options.length === 0) return false;

  const current = options.findIndex((option) => option.selected);
  const next = current + stepFor(direction);
  // Clamped, not wrapping. A wrap would let a held stick cycle a setting past
  // its end and back round without the user noticing it had moved at all.
  if (next < 0 || next >= options.length) return true;
  if (next === current) return true;

  return commit(select, HTMLSelectElement.prototype, options[next].value, ['input', 'change']);
}

function adaptRange(input: HTMLInputElement, direction: ControllerDirection): boolean {
  const min = Number.parseFloat(input.min || '0');
  const max = Number.parseFloat(input.max || '100');
  const step = Number.parseFloat(input.step || '1') || 1;
  const value = Number.parseFloat(input.value || '0');
  if (!Number.isFinite(min) || !Number.isFinite(max) || !Number.isFinite(value)) return false;

  const next = Math.min(max, Math.max(min, value + step * stepFor(direction)));
  if (next === value) return true;

  return commit(input, HTMLInputElement.prototype, String(next), ['input', 'change']);
}

function adaptNumber(input: HTMLInputElement, direction: ControllerDirection): boolean {
  const step = Number.parseFloat(input.step || '1') || 1;
  const value = Number.parseFloat(input.value || '0');
  if (!Number.isFinite(value)) return false;

  const min = input.min === '' ? Number.NEGATIVE_INFINITY : Number.parseFloat(input.min);
  const max = input.max === '' ? Number.POSITIVE_INFINITY : Number.parseFloat(input.max);
  const next = Math.min(max, Math.max(min, value + step * stepFor(direction)));
  if (next === value) return true;

  return commit(input, HTMLInputElement.prototype, String(next), ['input', 'change']);
}

/**
 * Whether the direction runs along the control's own axis.
 *
 * A slider in a column of settings has to give Up and Down back to the page, or
 * there is no way to leave it; it keeps Left and Right for itself. A dropdown
 * is the other way round, matching what those keys do natively.
 */
function ownsDirection(element: HTMLElement, direction: ControllerDirection): boolean {
  const horizontal = direction === 'left' || direction === 'right';
  if (element instanceof HTMLSelectElement) return !horizontal;
  if (element instanceof HTMLInputElement) {
    if (element.type === 'range') return horizontal;
    if (element.type === 'number') return !horizontal;
  }
  return false;
}

/**
 * Let a focused native control consume a navigation intent.
 *
 * Returns true when the control handled it — including when it was already at
 * the end of its range, because moving focus off a half-adjusted slider on the
 * press that was meant to nudge it is worse than doing nothing.
 */
export function adaptNavigate(element: Element | null, direction: ControllerDirection): boolean {
  if (!(element instanceof HTMLElement)) return false;
  if (element.hasAttribute('disabled') || element.getAttribute('aria-disabled') === 'true') return false;
  if (!ownsDirection(element, direction)) return false;

  if (element instanceof HTMLSelectElement) return adaptSelect(element, direction);
  if (element instanceof HTMLInputElement) {
    if (element.type === 'range') return adaptRange(element, direction);
    if (element.type === 'number') return adaptNumber(element, direction);
  }
  return false;
}

/**
 * Whether Accept should be swallowed rather than turned into a click.
 *
 * Clicking a select or a colour swatch is precisely what opens the OS widget
 * this module exists to avoid, so those absorb it. Everything else — buttons,
 * checkboxes, switches — still activates normally.
 */
export function adaptAccept(element: Element | null): boolean {
  if (!(element instanceof HTMLElement)) return false;
  if (element instanceof HTMLSelectElement) return true;
  if (element instanceof HTMLInputElement) {
    return element.type === 'color' || element.type === 'range' || element.type === 'file';
  }
  return false;
}
