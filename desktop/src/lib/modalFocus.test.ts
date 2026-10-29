import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { installModalFocus } from './modalFocus';

let stop: () => void;
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

function key(target: Element, k: string, shiftKey = false) {
  const event = new KeyboardEvent('keydown', { key: k, shiftKey, bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function mountPage(dialogHtml = '') {
  document.body.innerHTML = `
    <aside><button id="side">Sidebar</button></aside>
    <main><button id="opener">Install</button></main>
    ${dialogHtml}`;
}

const REVIEW = `
  <section role="dialog" aria-modal="true" id="dlg">
    <button id="x" data-modal-close="" aria-label="Close install panel">Close</button>
    <input id="mid" type="text" />
    <button id="last">Install</button>
  </section>`;

beforeEach(() => { stop = installModalFocus(document); });
afterEach(() => { stop(); document.body.innerHTML = ''; });

describe('hand-rolled modal dialogs', () => {
  it('moves focus into the dialog when it opens and back to the opener when it closes', async () => {
    mountPage();
    const opener = document.getElementById('opener')!;
    opener.focus();
    document.body.insertAdjacentHTML('beforeend', REVIEW);
    await flush();
    expect(document.activeElement?.id).toBe('x');

    document.getElementById('dlg')!.remove();
    await flush();
    expect(document.activeElement).toBe(opener);
  });

  it('keeps Tab and Shift+Tab inside the dialog', async () => {
    mountPage(REVIEW);
    await flush();
    const last = document.getElementById('last')!;
    last.focus();
    expect(key(last, 'Tab').defaultPrevented).toBe(true);
    expect(document.activeElement?.id).toBe('x');
    expect(key(document.activeElement!, 'Tab', true).defaultPrevented).toBe(true);
    expect(document.activeElement?.id).toBe('last');
  });

  it('pulls focus in when it has strayed to the page behind', async () => {
    mountPage(REVIEW);
    await flush();
    const side = document.getElementById('side')!;
    side.focus();
    key(side, 'Tab');
    expect(document.activeElement?.id).toBe('x');
  });

  it('presses the close action on Escape', async () => {
    mountPage(REVIEW);
    await flush();
    const onClose = vi.fn();
    document.getElementById('x')!.addEventListener('click', onClose);
    const event = key(document.getElementById('mid')!, 'Escape');
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(event.defaultPrevented).toBe(true);
  });

  it('ignores Escape when the key was already handled', async () => {
    mountPage(REVIEW);
    await flush();
    const onClose = vi.fn();
    document.getElementById('x')!.addEventListener('click', onClose);
    document.addEventListener('keydown', (e) => e.preventDefault(), { once: true, capture: true });
    key(document.getElementById('mid')!, 'Escape');
    expect(onClose).not.toHaveBeenCalled();
  });

  it('treats an inert (closed, still mounted) scrim as not open', async () => {
    mountPage(`<div inert id="scrim"><div role="dialog" aria-modal="true"><button id="in">Close</button></div></div>`);
    document.getElementById('opener')!.focus();
    await flush();
    expect(document.activeElement?.id).toBe('opener');
    const tab = key(document.getElementById('opener')!, 'Tab');
    expect(tab.defaultPrevented).toBe(false);
  });

  it('opens when an inert scrim becomes live', async () => {
    mountPage(`<div inert id="scrim"><div role="dialog" aria-modal="true"><button id="in">Close</button></div></div>`);
    document.getElementById('opener')!.focus();
    await flush();
    document.getElementById('scrim')!.removeAttribute('inert');
    await flush();
    expect(document.activeElement?.id).toBe('in');
  });

  it('leaves non-modal and self-managed dialogs alone', async () => {
    mountPage(`
      <aside role="dialog" aria-modal="false"><button id="a">Close</button></aside>
      <div role="dialog" aria-modal="true" data-modal-self-managed><button id="b">Close</button></div>`);
    document.getElementById('opener')!.focus();
    await flush();
    expect(document.activeElement?.id).toBe('opener');
  });

  it('does not fight a Radix-style dialog or popover opened above it', async () => {
    mountPage(REVIEW + `<div role="listbox"><div id="opt" tabindex="0">Option</div></div>`);
    await flush();
    const opt = document.getElementById('opt')!;
    opt.focus();
    expect(key(opt, 'Tab').defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(opt);
  });
});

describe('Escape picks only a pure dismissal', () => {
  it('presses a control whose whole label is a dismissal, allowing surrounding space', async () => {
    for (const label of ['Close', '  Cancel  ', 'Not now', 'Dismiss', '×', 'X']) {
      mountPage(`<section role="dialog" aria-modal="true"><input id="f" /><button id="act">${label}</button></section>`);
      await flush();
      const onClick = vi.fn();
      document.getElementById('act')!.addEventListener('click', onClick);
      key(document.getElementById('f')!, 'Escape');
      expect(onClick, label).toHaveBeenCalledTimes(1);
    }
  });

  it('never presses an action that only starts with a dismissal word', async () => {
    for (const label of ['Cancel download', 'Close instance', 'Close now', 'Dismiss all', 'Not now, install']) {
      mountPage(`<section role="dialog" aria-modal="true"><input id="f" /><button id="act">${label}</button></section>`);
      await flush();
      const onClick = vi.fn();
      document.getElementById('act')!.addEventListener('click', onClick);
      key(document.getElementById('f')!, 'Escape');
      expect(onClick, label).not.toHaveBeenCalled();
    }
  });

  it('prefers an explicit data-modal-close control, whatever its label', async () => {
    mountPage(`<section role="dialog" aria-modal="true"><input id="f" /><button id="act" data-modal-close="">Done</button></section>`);
    await flush();
    const onClick = vi.fn();
    document.getElementById('act')!.addEventListener('click', onClick);
    key(document.getElementById('f')!, 'Escape');
    expect(onClick).toHaveBeenCalledTimes(1);
  });
});

describe('what the observer watches', () => {
  it('watches open/close attributes and not class or style, which change every animation frame', () => {
    stop();
    const spy = vi.spyOn(MutationObserver.prototype, 'observe');
    stop = installModalFocus(document);
    const calls = spy.mock.calls;
    const options = calls[calls.length - 1][1] as MutationObserverInit;
    spy.mockRestore();
    expect(options.attributeFilter).toEqual(['inert', 'hidden', 'aria-hidden', 'aria-modal', 'role']);
  });
});

describe('the opener', () => {
  it('is still returned to when the dialog focused its own control before the observer ran', async () => {
    mountPage();
    const opener = document.getElementById('opener')!;
    opener.focus();
    document.body.insertAdjacentHTML('beforeend', REVIEW);
    // The dialog focuses its own control before the microtask sync runs.
    document.getElementById('x')!.focus();
    await flush();
    expect(document.activeElement?.id).toBe('x');

    document.getElementById('dlg')!.remove();
    await flush();
    expect(document.activeElement).toBe(opener);
  });

  it('is not restored to a stale element after focus went to bare page', async () => {
    mountPage();
    const opener = document.getElementById('opener')!;
    opener.focus();
    // Focus leaves for nowhere, as a click on bare page does.
    opener.blur();
    document.body.insertAdjacentHTML('beforeend', REVIEW);
    await flush();
    document.getElementById('dlg')!.remove();
    await flush();
    expect(document.activeElement).not.toBe(opener);
  });
});
