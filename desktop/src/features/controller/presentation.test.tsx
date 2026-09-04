import { render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ControllerProvider } from './ControllerProvider';
import { COUCH_PRESENTATION, PRESENTATION_ATTRIBUTE, setCouchPresentation } from './presentation';
import { GAMEPAD_MODALITY, INPUT_MODALITY_ATTRIBUTE } from './inputModality';

const harness = vi.hoisted(() => ({ connected: false }));

vi.mock('../../lib/useGamepad', () => ({
  useGamepad: () => ({
    connected: harness.connected,
    gamepadCount: harness.connected ? 1 : 0,
  }),
}));

beforeEach(() => {
  harness.connected = false;
  document.documentElement.removeAttribute(PRESENTATION_ATTRIBUTE);
  document.documentElement.removeAttribute(INPUT_MODALITY_ATTRIBUTE);
});

afterEach(() => {
  document.documentElement.removeAttribute(PRESENTATION_ATTRIBUTE);
  document.documentElement.removeAttribute(INPUT_MODALITY_ATTRIBUTE);
});

const presentation = () => document.documentElement.getAttribute(PRESENTATION_ATTRIBUTE);

describe('couch presentation', () => {
  it('stays off when no controller is present', () => {
    render(<ControllerProvider><div /></ControllerProvider>);

    expect(presentation()).toBeNull();
  });

  it('sizes the app for a controller once one is in play', () => {
    harness.connected = true;
    render(<ControllerProvider><div /></ControllerProvider>);

    expect(presentation()).toBe(COUCH_PRESENTATION);
  });

  it('puts the layout back when the provider goes away', () => {
    harness.connected = true;
    const view = render(<ControllerProvider><div /></ControllerProvider>);
    expect(presentation()).toBe(COUCH_PRESENTATION);

    view.unmount();

    expect(presentation()).toBeNull();
  });

  /**
   * The distinction that matters. Modality flips the instant a mouse is
   * touched, which is right for a focus ring; resizing the entire interface
   * every time a hand moves between pad and mouse would be unusable, so layout
   * follows presence instead.
   */
  it('does not follow input modality', () => {
    harness.connected = true;
    render(<ControllerProvider><div /></ControllerProvider>);

    document.documentElement.setAttribute(INPUT_MODALITY_ATTRIBUTE, GAMEPAD_MODALITY);
    document.documentElement.removeAttribute(INPUT_MODALITY_ATTRIBUTE);

    expect(presentation()).toBe(COUCH_PRESENTATION);
  });
});

describe('setCouchPresentation', () => {
  it('is idempotent in both directions', () => {
    setCouchPresentation(true);
    setCouchPresentation(true);
    expect(presentation()).toBe(COUCH_PRESENTATION);

    setCouchPresentation(false);
    setCouchPresentation(false);
    expect(presentation()).toBeNull();
  });
});
