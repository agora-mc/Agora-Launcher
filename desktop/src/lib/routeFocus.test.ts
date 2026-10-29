import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { GAMEPAD_MODALITY, INPUT_MODALITY_ATTRIBUTE } from '../features/controller/inputModality';
import { focusFollowsRoute, moveFocusForRoute, routeFocusTarget } from './routeFocus';

function page(html: string): { shell: HTMLElement; main: HTMLElement; sidebar: HTMLElement } {
  document.body.innerHTML = `
    <div id="shell">
      <aside id="sidebar"><button id="nav-home">Home</button></aside>
      <main id="content" tabindex="-1">${html}</main>
    </div>
    <div id="dialog"><button id="dialog-button">Inside a dialog</button></div>
  `;
  return {
    shell: document.getElementById('shell') as HTMLElement,
    main: document.getElementById('content') as HTMLElement,
    sidebar: document.getElementById('sidebar') as HTMLElement,
  };
}

beforeEach(() => {
  document.documentElement.removeAttribute(INPUT_MODALITY_ATTRIBUTE);
});

afterEach(() => {
  document.body.innerHTML = '';
  document.documentElement.removeAttribute(INPUT_MODALITY_ATTRIBUTE);
});

describe('routeFocusTarget', () => {
  it('returns the first heading and makes it focusable without joining the Tab order', () => {
    const { main } = page('<h1 id="title">Browse</h1><h1 id="later">Other</h1>');
    const target = routeFocusTarget(main);
    expect(target.id).toBe('title');
    expect(target.getAttribute('tabindex')).toBe('-1');
  });

  it('uses an h2 page title when the page has no h1, which is how most pages title themselves', () => {
    const { main } = page('<p>intro</p><h2 id="title">My Instances</h2><h2>Section</h2>');
    expect(routeFocusTarget(main).id).toBe('title');
  });

  it('skips a heading inside a hidden subtree', () => {
    const { main } = page('<div hidden><h1 id="hidden-title">Mod</h1></div><h1 id="visible">Instances</h1>');
    expect(routeFocusTarget(main).id).toBe('visible');
  });

  it('skips a heading inside a Tailwind hidden element', () => {
    const { main } = page('<div class="hidden"><h1 id="hidden-title">Mod</h1></div><h1 id="visible">Home</h1>');
    expect(routeFocusTarget(main).id).toBe('visible');
  });

  it('falls back to the content area when the page has no heading', () => {
    const { main } = page('<p>Nothing titled here.</p>');
    expect(routeFocusTarget(main)).toBe(main);
  });

  it('falls back to the content area when every heading is hidden', () => {
    const { main } = page('<div hidden><h1>Gone</h1></div>');
    expect(routeFocusTarget(main)).toBe(main);
  });

  it('keeps a tabindex the page already set', () => {
    const { main } = page('<h1 id="t" tabindex="0">Title</h1>');
    expect(routeFocusTarget(main).getAttribute('tabindex')).toBe('0');
  });
});

describe('focusFollowsRoute', () => {
  it('follows focus that is on the body or lost entirely', () => {
    const { main, sidebar } = page('<h1>x</h1>');
    expect(focusFollowsRoute(null, main, sidebar)).toBe(true);
    expect(focusFollowsRoute(document.body, main, sidebar)).toBe(true);
  });

  it('follows focus on a sidebar button and on content inside the page', () => {
    const { main, sidebar } = page('<button id="card">Open</button>');
    expect(focusFollowsRoute(document.getElementById('nav-home'), main, sidebar)).toBe(true);
    expect(focusFollowsRoute(document.getElementById('card'), main, sidebar)).toBe(true);
  });

  it('leaves focus alone inside a dialog outside the shell', () => {
    const { main, sidebar } = page('<h1>x</h1>');
    expect(focusFollowsRoute(document.getElementById('dialog-button'), main, sidebar)).toBe(false);
  });
});

describe('moveFocusForRoute', () => {
  it('moves focus to the new page heading when the user is in the shell', () => {
    const { main, sidebar } = page('<h1 id="title">Instances</h1>');
    document.getElementById('nav-home')?.focus();
    moveFocusForRoute(main, sidebar);
    expect(document.activeElement?.id).toBe('title');
  });

  it('does not steal focus from a dialog the user is working in', () => {
    const { main, sidebar } = page('<h1 id="title">Instances</h1>');
    const dialogButton = document.getElementById('dialog-button') as HTMLElement;
    dialogButton.focus();
    moveFocusForRoute(main, sidebar);
    expect(document.activeElement).toBe(dialogButton);
  });

  it('leaves controller users to focus memory', () => {
    const { main, sidebar } = page('<h1 id="title">Instances</h1>');
    document.documentElement.setAttribute(INPUT_MODALITY_ATTRIBUTE, GAMEPAD_MODALITY);
    document.getElementById('nav-home')?.focus();
    moveFocusForRoute(main, sidebar);
    expect(document.activeElement?.id).toBe('nav-home');
  });
});
