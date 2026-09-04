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
 * The fix is to never open the OS widget at all. A slider steps by its own
 * increment, and a select hands its options to an in-app overlay instead of the
 * system popup — see `SelectOverlay`. Neither ever opens something the
 * controller would be locked out of, and no page markup had to change.
 *
 * Selects deliberately own no direction. An earlier version cycled their
 * options with up and down, which read well in isolation and was miserable in
 * practice: in a column of settings every dropdown became a trap that vertical
 * movement could not get past.
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

/**
 * Write a value the way a user would, so React's tracker notices.
 *
 * Exported because every controller-driven write to a form control has to go
 * through this, including the on-screen keyboard. Assigning `.value` directly
 * is the silent-failure path: the DOM updates, `onChange` never fires, and a
 * controlled component renders the old value straight back.
 */
export function setNativeValue(
  element: HTMLElement,
  prototype: object,
  value: string,
  events: string[] = ['input', 'change'],
): boolean {
  return commit(element, prototype, value, events);
}

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

/** The options a controller may choose between, in order. */
export interface SelectChoice {
  value: string;
  label: string;
  index: number;
}

export function selectChoices(select: HTMLSelectElement): SelectChoice[] {
  return Array.from(select.options)
    .map((option, index) => ({ option, index }))
    .filter(({ option }) => !option.disabled)
    .map(({ option, index }) => ({
      value: option.value,
      label: option.label || option.text || option.value,
      index,
    }));
}

export function selectedChoiceIndex(select: HTMLSelectElement): number {
  const choices = selectChoices(select);
  const found = choices.findIndex((choice) => choice.index === select.selectedIndex);
  return found >= 0 ? found : 0;
}

/** Commit a chosen option the way a user would, so React's tracker notices. */
export function commitSelectValue(select: HTMLSelectElement, value: string): boolean {
  if (select.value === value) return true;
  return commit(select, HTMLSelectElement.prototype, value, ['input', 'change']);
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

  if (element instanceof HTMLInputElement) {
    if (element.type === 'range') return adaptRange(element, direction);
    if (element.type === 'number') return adaptNumber(element, direction);
  }
  return false;
}

/**
 * Whether Accept should be swallowed rather than turned into a click.
 *
 * A range has nothing to activate and a file input opens an OS window nothing
 * can steer, so those absorb it. Selects and colour swatches do not: the
 * provider opens an in-app overlay for each. Everything else — buttons,
 * checkboxes, switches — still activates normally.
 */
export function adaptAccept(element: Element | null): boolean {
  if (!(element instanceof HTMLElement)) return false;
  if (element instanceof HTMLInputElement) {
    // `color` is absent on purpose: the provider opens a controller-operable
    // picker for it. Absorbing Accept here would make colour settings inert.
    return element.type === 'range' || element.type === 'file';
  }
  return false;
}
