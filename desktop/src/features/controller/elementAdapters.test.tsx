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
import {
  adaptAccept,
  adaptNavigate,
  commitSelectValue,
  selectChoices,
  selectedChoiceIndex,
} from './elementAdapters';

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
  /**
   * Selects deliberately own no direction. An earlier version cycled their
   * options with up and down, which read well in isolation and was miserable in
   * a column of settings: every dropdown became a trap vertical movement could
   * not get past. Choosing happens in an overlay on Accept instead.
   */
  it('does not consume any direction, so focus can move past it', () => {
    render(<ControlledSelect />);
    const select = screen.getByLabelText('pick');

    for (const direction of ['up', 'down', 'left', 'right'] as const) {
      expect(adaptNavigate(select, direction)).toBe(false);
    }
    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('b');
  });

  it('lists the choices a controller may pick between', () => {
    render(<ControlledSelect />);
    const select = screen.getByLabelText('pick') as HTMLSelectElement;

    expect(selectChoices(select).map((choice) => choice.value)).toEqual(['a', 'b', 'c']);
    expect(selectedChoiceIndex(select)).toBe(1);
  });

  it('omits disabled options from the choices', () => {
    render(
      <select aria-label="pick" defaultValue="a">
        <option value="a">A</option>
        <option value="b" disabled>B</option>
        <option value="c">C</option>
      </select>,
    );
    const select = screen.getByLabelText('pick') as HTMLSelectElement;

    expect(selectChoices(select).map((choice) => choice.value)).toEqual(['a', 'c']);
  });

  it('reports an empty select without throwing', () => {
    render(<select aria-label="empty" />);
    const select = screen.getByLabelText('empty') as HTMLSelectElement;

    expect(selectChoices(select)).toEqual([]);
    expect(selectedChoiceIndex(select)).toBe(0);
  });

  it('commits a chosen value through React rather than only the DOM', () => {
    render(<ControlledSelect />);
    const select = screen.getByLabelText('pick') as HTMLSelectElement;

    expect(commitSelectValue(select, 'c')).toBe(true);

    expect(select.value).toBe('c');
    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('c');
  });

  it('treats committing the current value as a no-op', () => {
    render(<ControlledSelect />);
    const select = screen.getByLabelText('pick') as HTMLSelectElement;

    expect(commitSelectValue(select, 'b')).toBe(true);
    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('b');
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
        <input aria-label="c" type="color" defaultValue="#ffffff" />
        <input aria-label="f" type="file" />
      </>,
    );

    expect(adaptAccept(screen.getByLabelText('c'))).toBe(true);
    expect(adaptAccept(screen.getByLabelText('f'))).toBe(true);
  });

  /** The provider opens the select overlay for these, so Accept must not be
   *  absorbed here — absorbing it would make dropdowns do nothing at all. */
  it('is not swallowed for a select, which opens the overlay instead', () => {
    render(<select aria-label="s"><option value="a">A</option></select>);

    expect(adaptAccept(screen.getByLabelText('s'))).toBe(false);
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
   * component re-renders its old value. If this fails, a write has stopped
   * going through the prototype's native setter.
   */
  it('keeps a controlled select on its new value after a re-render', () => {
    render(<ControlledSelect />);
    const select = screen.getByLabelText('pick') as HTMLSelectElement;

    commitSelectValue(select, 'c');
    fireEvent.blur(select);

    expect(select.value).toBe('c');
    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('c');
  });

  it('keeps a controlled slider on its new value after a re-render', () => {
    render(<ControlledRange />);
    const slider = screen.getByLabelText('level') as HTMLInputElement;

    adaptNavigate(slider, 'right');
    fireEvent.blur(slider);

    expect(slider.value).toBe('60');
    expect(screen.getByRole('status', { hidden: true }).textContent).toBe('60');
  });
});
