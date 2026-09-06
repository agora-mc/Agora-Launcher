import { useRef } from 'react';
import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { GamepadIntent } from '../../lib/useGamepad';
import { ControllerProvider } from './ControllerProvider';
import { useControllerLayer } from './useControllerLayer';

const harness = vi.hoisted(() => ({
  onIntent: undefined as ((intent: GamepadIntent) => void) | undefined,
}));

vi.mock('../../lib/useGamepad', () => ({
  useGamepad: (options: { onIntent?: (intent: GamepadIntent) => void }) => {
    harness.onIntent = options.onIntent;
    return { connected: true, gamepadCount: 1 };
  },
}));

const originalScrollIntoView = HTMLElement.prototype.scrollIntoView;

beforeEach(() => {
  harness.onIntent = undefined;
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    writable: true,
    value: vi.fn(),
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  if (originalScrollIntoView) {
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
      configurable: true,
      writable: true,
      value: originalScrollIntoView,
    });
  } else {
    Reflect.deleteProperty(HTMLElement.prototype, 'scrollIntoView');
  }
});

function send(intent: GamepadIntent): void {
  act(() => harness.onIntent?.(intent));
}

function move(direction: 'up' | 'down' | 'left' | 'right'): void {
  send({ type: 'direction', direction });
}

function scroll(direction: 'up' | 'down' | 'left' | 'right'): void {
  send({ type: 'stick', direction });
}

function rect(left: number, top: number, right: number, bottom: number): DOMRect {
  return { left, top, right, bottom, width: right - left, height: bottom - top } as DOMRect;
}

function setRect(element: HTMLElement, value: DOMRect): void {
  vi.spyOn(element, 'getBoundingClientRect').mockReturnValue(value);
}

function setScrollMetrics(element: HTMLElement, metrics: {
  clientHeight?: number;
  clientWidth?: number;
  scrollHeight?: number;
  scrollWidth?: number;
  scrollTop?: number;
  scrollLeft?: number;
} = {}): void {
  for (const [name, value] of Object.entries({
    clientHeight: 100,
    clientWidth: 100,
    scrollHeight: 200,
    scrollWidth: 200,
    scrollTop: 0,
    scrollLeft: 0,
    ...metrics,
  })) {
    Object.defineProperty(element, name, { configurable: true, writable: true, value });
  }
}

function ButtonLayer({
  wrapperClassName,
}: {
  wrapperClassName?: string;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  useControllerLayer({ rootRef });
  return (
    <div className={wrapperClassName} data-testid="scrollport">
      <div ref={rootRef}>
        <button type="button">first</button>
        <button type="button">second</button>
      </div>
    </div>
  );
}

describe('controller navigation geometry', () => {
  it('uses the aligned geometric candidate instead of document order', () => {
    render(<ControllerProvider><ButtonLayer /></ControllerProvider>);
    const first = screen.getByText('first');
    const second = screen.getByText('second');

    setRect(first, rect(0, 0, 40, 40));
    setRect(second, rect(0, 60, 40, 100));
    move('down');
    move('down');

    expect(document.activeElement).toBe(second);
  });

  it('focuses without a browser focus scroll, then requests nearest scrolling', () => {
    render(<ControllerProvider><ButtonLayer /></ControllerProvider>);
    const first = screen.getByText('first');
    const second = screen.getByText('second');
    setRect(first, rect(0, 0, 40, 40));
    setRect(second, rect(0, 60, 40, 100));

    const focus = vi.spyOn(HTMLElement.prototype, 'focus');
    move('down');
    move('down');

    expect(focus).toHaveBeenLastCalledWith({ preventScroll: true });
    expect(HTMLElement.prototype.scrollIntoView).toHaveBeenLastCalledWith({
      block: 'nearest',
      inline: 'nearest',
    });
  });

  it('scrolls the focused element’s nearest scrollport and retries geometry', () => {
    render(<ControllerProvider><ButtonLayer /></ControllerProvider>);
    const scrollport = screen.getByTestId('scrollport');
    const first = screen.getByText('first');
    const second = screen.getByText('second');
    setScrollMetrics(scrollport);
    scrollport.style.overflowY = 'auto';

    setRect(first, rect(0, 0, 40, 40));
    const secondRect = vi.spyOn(second, 'getBoundingClientRect');
    secondRect.mockImplementation(() => scrollport.scrollTop > 0
      ? rect(0, 60, 40, 100)
      : rect(0, -100, 40, -60));
    const scrollBy = vi.fn(({ top }: ScrollToOptions) => {
      scrollport.scrollTop += top ?? 0;
    });
    Object.defineProperty(scrollport, 'scrollBy', { configurable: true, value: scrollBy });

    move('down');
    move('down');

    expect(scrollBy).toHaveBeenCalledWith({ behavior: 'auto', left: 0, top: 100 });
    expect(document.activeElement).toBe(second);
  });
});

describe('default controller scrolling', () => {
  it('scrolls the focused element’s nearest scrollport without moving focus', () => {
    render(<ControllerProvider><ButtonLayer /></ControllerProvider>);
    const scrollport = screen.getByTestId('scrollport');
    const first = screen.getByText('first');
    setScrollMetrics(scrollport);
    scrollport.style.overflowY = 'auto';
    const scrollBy = vi.fn();
    Object.defineProperty(scrollport, 'scrollBy', { configurable: true, value: scrollBy });

    first.focus();
    scroll('down');

    expect(document.activeElement).toBe(first);
    expect(scrollBy).toHaveBeenCalledWith({ behavior: 'auto', left: 0, top: 100 });
  });
});
