/**
 * These controls are the difference between reachable and operable. A select
 * that a controller can focus but not change is not controller support.
 *
 * The subtle requirement is React's value tracker: assigning `element.value`
 * updates the DOM but skips `onChange`, so a controlled component would render
 * the old value straight back and the setting would appear not to move. Every
 * case below therefore asserts against a *controlled* React component, not a
 * bare DOM node.
 */
import { useState } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { adaptAccept, adaptNavigate } from './elementAdapters';

function ControlledSelect({ initial = 'b' }: { initial?: string }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <select aria-label="pick" value={value} onChange={(e) => setValue(e.target.value)}>
        <option value="a">A</option>
        <option value="b">B</option>
        <option value="c">C</option>
      </select>
      <output>{value}</output>
    </>
  );
}

function ControlledRange({ initial = 50 }: { initial?: number }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <input
        aria-label="level"
        type="range"
        min="0"
        max="100"
        step="10"
        value={value}
        onChange={(e) => setValue(Number(e.target.value))}
      />
      <output>{value}</output>
    </>
  );
}

describe('native select', () => {
  it('moves a controlled value down the list', () => {
    render(<ControlledSelect />);
    const select = screen.getByLabelText('pick');

    expect(adaptNavigate(select, 'down')).toBe(true);

    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('c');
  });

  it('moves back up the list', () => {
    render(<ControlledSelect />);

    adaptNavigate(screen.getByLabelText('pick'), 'up');

    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('a');
  });

  it('clamps at the ends instead of wrapping', () => {
    render(<ControlledSelect initial="c" />);
    const select = screen.getByLabelText('pick');

    expect(adaptNavigate(select, 'down')).toBe(true);
    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('c');
  });

  it('gives left and right back to the page so focus can leave', () => {
    render(<ControlledSelect />);
    const select = screen.getByLabelText('pick');

    expect(adaptNavigate(select, 'left')).toBe(false);
    expect(adaptNavigate(select, 'right')).toBe(false);
  });

  it('skips disabled options', () => {
    function WithDisabled() {
      const [value, setValue] = useState('a');
      return (
        <>
          <select aria-label="pick" value={value} onChange={(e) => setValue(e.target.value)}>
            <option value="a">A</option>
            <option value="b" disabled>B</option>
            <option value="c">C</option>
          </select>
          <output>{value}</output>
        </>
      );
    }
    render(<WithDisabled />);

    adaptNavigate(screen.getByLabelText('pick'), 'down');

    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('c');
  });

  it('leaves a multi-select to ordinary focus movement', () => {
    render(
      <select aria-label="many" multiple defaultValue={['a']}>
        <option value="a">A</option>
        <option value="b">B</option>
      </select>,
    );

    expect(adaptNavigate(screen.getByLabelText('many'), 'down')).toBe(false);
  });

  it('does nothing to a disabled select', () => {
    render(
      <select aria-label="off" disabled defaultValue="a">
        <option value="a">A</option>
        <option value="b">B</option>
      </select>,
    );

    expect(adaptNavigate(screen.getByLabelText('off'), 'down')).toBe(false);
  });

  it('survives a select with no options at all', () => {
    render(<select aria-label="empty" />);

    expect(adaptNavigate(screen.getByLabelText('empty'), 'down')).toBe(false);
  });
});

describe('range slider', () => {
  it('steps a controlled value by its own increment', () => {
    render(<ControlledRange />);

    expect(adaptNavigate(screen.getByLabelText('level'), 'right')).toBe(true);
    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('60');
  });

  it('steps backwards', () => {
    render(<ControlledRange />);

    adaptNavigate(screen.getByLabelText('level'), 'left');

    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('40');
  });

  it('clamps at its maximum', () => {
    render(<ControlledRange initial={100} />);

    expect(adaptNavigate(screen.getByLabelText('level'), 'right')).toBe(true);
    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('100');
  });

  it('gives up and down back so focus can leave a column of settings', () => {
    render(<ControlledRange />);
    const slider = screen.getByLabelText('level');

    expect(adaptNavigate(slider, 'up')).toBe(false);
    expect(adaptNavigate(slider, 'down')).toBe(false);
  });
});

describe('accept', () => {
  it('is swallowed by controls that would open an OS widget', () => {
    render(
      <>
        <select aria-label="s"><option value="a">A</option></select>
        <input aria-label="c" type="color" defaultValue="#ffffff" />
        <input aria-label="f" type="file" />
      </>,
    );

    expect(adaptAccept(screen.getByLabelText('s'))).toBe(true);
    expect(adaptAccept(screen.getByLabelText('c'))).toBe(true);
    expect(adaptAccept(screen.getByLabelText('f'))).toBe(true);
  });

  it('still activates ordinary controls', () => {
    render(
      <>
        <button type="button">go</button>
        <input aria-label="cb" type="checkbox" />
        <input aria-label="t" type="text" />
      </>,
    );

    expect(adaptAccept(screen.getByText('go'))).toBe(false);
    expect(adaptAccept(screen.getByLabelText('cb'))).toBe(false);
    expect(adaptAccept(screen.getByLabelText('t'))).toBe(false);
  });

  it('handles a null element', () => {
    expect(adaptAccept(null)).toBe(false);
    expect(adaptNavigate(null, 'down')).toBe(false);
  });
});

describe('React value tracking', () => {
  /**
   * The regression guard. Setting `.value` directly leaves React's tracker
   * believing nothing changed, so `onChange` never fires and a controlled
   * component re-renders its old value. If this test fails, the adapter has
   * stopped going through the prototype's native setter.
   */
  it('fires onChange on a controlled select rather than only moving the DOM', () => {
    render(<ControlledSelect />);
    const select = screen.getByLabelText('pick') as HTMLSelectElement;

    adaptNavigate(select, 'down');

    expect(select.value).toBe('c');
    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('c');
  });

  it('keeps a controlled select from snapping back after a re-render', () => {
    render(<ControlledSelect />);
    const select = screen.getByLabelText('pick') as HTMLSelectElement;

    adaptNavigate(select, 'down');
    fireEvent.blur(select);

    expect(select.value).toBe('c');
  });
});
