/**
 * One shared modal behaviour for every hand-rolled dialog.
 *
 * Radix dialogs already trap focus, close on Escape and give focus back. The
 * app also has a dozen dialogs built by hand: a `div` with `role="dialog"` and
 * `aria-modal="true"` over a scrim. Declaring a dialog modal is a promise to
 * assistive technology that the rest of the page is out of reach, and those
 * dialogs did not keep it: focus stayed on the page behind, Tab walked the
 * sidebar, and Escape did nothing.
 *
 * Rather than ask each of them to call a hook (and have the interactive
 * feature, which may not import app modules, carry its own copy), this watches
 * the document for `[role="dialog"][aria-modal="true"]` and gives whichever is
 * open on top the behaviour:
 *
 *  - focus moves into it when it opens (`[data-autofocus]` first, else the
 *    first tabbable control, else the dialog itself);
 *  - Tab and Shift+Tab wrap inside it;
 *  - Escape presses its close action (`[data-modal-close]`, else a button
 *    labelled Close / Cancel / Not now) when it has one;
 *  - focus returns to what had it before, once it closes.
 *
 * "Open" means attached, not `inert` / `hidden` / `aria-hidden`, and not
 * `visibility: hidden`. Scrims that stay mounted and fade out must set
 * `inert` while closed, which also stops Tab from reaching them.
 *
 * Opt out with `data-modal-self-managed` (the controller overlays own their
 * focus). Dialogs without `aria-modal="true"` (the tour card, the install
 * progress panel) are non-modal on purpose and are never touched.
 */

const MODAL_SELECTOR = '[role="dialog"][aria-modal="true"]:not([data-modal-self-managed])';

/** Anything that is its own layer above the page and handles its own keys. */
const FOREIGN_LAYER_SELECTOR = [
  '[role="dialog"]',
  '[role="alertdialog"]',
  '[role="listbox"]',
  '[role="menu"]',
  '[data-radix-popper-content-wrapper]',
  '[data-modal-self-managed]',
].join(',');

const TABBABLE_SELECTOR = [
  'a[href]',
  'button',
  'input',
  'select',
  'textarea',
  'summary',
  '[tabindex]',
  '[contenteditable=""]',
  '[contenteditable="true"]',
].join(',');

const CLOSE_LABEL = /^(×|✕|x$|close\b|cancel\b|not now\b|dismiss\b)/i;

function isDisplayed(el: HTMLElement): boolean {
  if (el.closest('[inert],[hidden],[aria-hidden="true"]')) return false;
  const style = el.ownerDocument.defaultView?.getComputedStyle(el);
  return !style || (style.visibility !== 'hidden' && style.display !== 'none');
}

export function isModalOpen(dialog: HTMLElement): boolean {
  return dialog.isConnected && isDisplayed(dialog);
}

export function tabbablesIn(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(TABBABLE_SELECTOR)).filter((el) => {
    if (el.tabIndex < 0) return false;
    if ((el as HTMLButtonElement).disabled) return false;
    if (el instanceof HTMLInputElement && el.type === 'hidden') return false;
    return isDisplayed(el);
  });
}

function closeActionOf(dialog: HTMLElement): HTMLElement | null {
  const explicit = dialog.querySelector<HTMLElement>('[data-modal-close]');
  if (explicit) return explicit;
  // A dialog's own labelled button, not a "Close" buried in a nested list.
  const buttons = dialog.querySelectorAll<HTMLElement>('button:not([disabled])');
  for (const button of buttons) {
    const label = (button.getAttribute('aria-label') ?? button.textContent ?? '').trim();
    if (label && CLOSE_LABEL.test(label) && isDisplayed(button)) return button;
  }
  return null;
}

function openModals(doc: Document): HTMLElement[] {
  return Array.from(doc.querySelectorAll<HTMLElement>(MODAL_SELECTOR)).filter(isModalOpen);
}

/** The modal that owns the keyboard: the last one in document order. */
export function topModal(doc: Document): HTMLElement | null {
  const open = openModals(doc);
  return open[open.length - 1] ?? null;
}

/** True when `node` sits in some other layer (Radix dialog, menu, popover). */
function inForeignLayer(node: Element | null, top: HTMLElement): boolean {
  if (!node || top.contains(node)) return false;
  return node.closest(FOREIGN_LAYER_SELECTOR) !== null;
}

function focusInto(dialog: HTMLElement): void {
  if (dialog.contains(dialog.ownerDocument.activeElement)) return;
  const preferred = dialog.querySelector<HTMLElement>('[data-autofocus]');
  const target = preferred && isDisplayed(preferred) ? preferred : tabbablesIn(dialog)[0];
  if (target) {
    target.focus({ preventScroll: true });
    return;
  }
  if (!dialog.hasAttribute('tabindex')) dialog.setAttribute('tabindex', '-1');
  dialog.focus({ preventScroll: true });
}

export function handleModalKeyDown(event: KeyboardEvent, doc: Document): void {
  if (event.defaultPrevented) return;
  if (event.key !== 'Tab' && event.key !== 'Escape') return;
  const top = topModal(doc);
  if (!top) return;
  const active = doc.activeElement;
  if (inForeignLayer(active, top)) return;
  const target = event.target instanceof Element ? event.target : null;
  if (inForeignLayer(target, top)) return;

  if (event.key === 'Escape') {
    const close = closeActionOf(top);
    if (!close) return;
    event.preventDefault();
    close.click();
    return;
  }

  const tabbables = tabbablesIn(top);
  if (tabbables.length === 0) {
    event.preventDefault();
    focusInto(top);
    return;
  }
  const first = tabbables[0];
  const last = tabbables[tabbables.length - 1];
  const inside = active !== null && top.contains(active) && active !== top;
  if (!inside) {
    event.preventDefault();
    (event.shiftKey ? last : first).focus();
  } else if (event.shiftKey && active === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && active === last) {
    event.preventDefault();
    first.focus();
  }
}

interface Tracked {
  opener: HTMLElement | null;
}

/**
 * Start watching `doc`. Returns a function that stops. Safe to call once for
 * the whole app; it costs one cheap selector query per DOM mutation batch.
 */
export function installModalFocus(doc: Document = document): () => void {
  const tracked = new Map<HTMLElement, Tracked>();
  let scheduled = false;

  const sync = () => {
    scheduled = false;
    const open = new Set(openModals(doc));

    for (const dialog of open) {
      if (tracked.has(dialog)) continue;
      const active = doc.activeElement instanceof HTMLElement ? doc.activeElement : null;
      // Focus already inside (the dialog focused something itself): keep it,
      // and remember what had focus only if that was something real.
      const opener = active && active !== doc.body && !dialog.contains(active) ? active : null;
      tracked.set(dialog, { opener });
      focusInto(dialog);
    }

    for (const [dialog, info] of Array.from(tracked)) {
      if (open.has(dialog)) continue;
      tracked.delete(dialog);
      const active = doc.activeElement;
      const lost = !active || active === doc.body || dialog.contains(active);
      if (lost && info.opener && info.opener.isConnected && isDisplayed(info.opener)) {
        info.opener.focus({ preventScroll: true });
      }
    }
  };

  const schedule = () => {
    if (scheduled) return;
    scheduled = true;
    queueMicrotask(sync);
  };

  const observer = new MutationObserver(schedule);
  observer.observe(doc.documentElement, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['inert', 'hidden', 'aria-hidden', 'aria-modal', 'role', 'class', 'style'],
  });
  const onKeyDown = (event: KeyboardEvent) => handleModalKeyDown(event, doc);
  doc.addEventListener('keydown', onKeyDown);
  schedule();

  return () => {
    observer.disconnect();
    doc.removeEventListener('keydown', onKeyDown);
    tracked.clear();
  };
}
