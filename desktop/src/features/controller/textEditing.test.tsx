import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import {
  clearField,
  deleteBackwards,
  insertText,
  isEditableField,
  moveCaret,
  type EditableField,
} from './textEditing';

/** Controlled, because the whole point is that React's onChange must fire. */
function Field({ initial = '', ...rest }: { initial?: string } & Record<string, unknown>) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <input aria-label="field" value={value} onChange={(e) => setValue(e.target.value)} {...rest} />
      <output>{value}</output>
    </>
  );
}

function field(): EditableField {
  return screen.getByLabelText('field') as EditableField;
}

const rendered = () => screen.getByRole('status', { hidden: true }).textContent;

function caretAt(target: EditableField, position: number) {
  target.setSelectionRange(position, position);
}

describe('insertText', () => {
  it('appends when the caret is at the end', () => {
    render(<Field initial="ab" />);
    caretAt(field(), 2);

    insertText(field(), 'c');

    expect(rendered()).toBe('abc');
  });

  it('inserts at the caret rather than the end', () => {
    render(<Field initial="ac" />);
    caretAt(field(), 1);

    insertText(field(), 'b');

    expect(rendered()).toBe('abc');
    expect(field().selectionStart).toBe(2);
  });

  it('replaces a selected range', () => {
    render(<Field initial="axxxc" />);
    field().setSelectionRange(1, 4);

    insertText(field(), 'b');

    expect(rendered()).toBe('abc');
  });

  it('respects maxLength', () => {
    render(<Field initial="abc" maxLength={4} />);
    caretAt(field(), 3);

    insertText(field(), 'de');

    expect(rendered()).toBe('abcd');
  });

  it('refuses to overflow a full field', () => {
    render(<Field initial="abcd" maxLength={4} />);
    caretAt(field(), 4);

    expect(insertText(field(), 'e')).toBe(false);
    expect(rendered()).toBe('abcd');
  });

  it('ignores an empty insert', () => {
    render(<Field initial="ab" />);

    expect(insertText(field(), '')).toBe(false);
  });
});

describe('deleteBackwards', () => {
  it('removes the character before the caret', () => {
    render(<Field initial="abc" />);
    caretAt(field(), 2);

    deleteBackwards(field());

    expect(rendered()).toBe('ac');
    expect(field().selectionStart).toBe(1);
  });

  it('removes a selection in one press', () => {
    render(<Field initial="abcd" />);
    field().setSelectionRange(1, 3);

    deleteBackwards(field());

    expect(rendered()).toBe('ad');
  });

  it('does nothing at the start of the value', () => {
    render(<Field initial="abc" />);
    caretAt(field(), 0);

    expect(deleteBackwards(field())).toBe(false);
    expect(rendered()).toBe('abc');
  });

  it('does nothing to an empty field', () => {
    render(<Field initial="" />);

    expect(deleteBackwards(field())).toBe(false);
  });

  /** Half an emoji is a broken character, not a smaller one. */
  it('removes a whole astral character rather than one code unit', () => {
    render(<Field initial="a😀" />);
    caretAt(field(), 3);

    deleteBackwards(field());

    expect(rendered()).toBe('a');
  });
});

describe('moveCaret', () => {
  it('steps left and right', () => {
    render(<Field initial="abc" />);
    caretAt(field(), 2);

    moveCaret(field(), 'left');
    expect(field().selectionStart).toBe(1);

    moveCaret(field(), 'right');
    expect(field().selectionStart).toBe(2);
  });

  it('stops at each end', () => {
    render(<Field initial="ab" />);
    caretAt(field(), 0);
    expect(moveCaret(field(), 'left')).toBe(false);

    caretAt(field(), 2);
    expect(moveCaret(field(), 'right')).toBe(false);
  });

  it('steps over a whole astral character', () => {
    render(<Field initial="😀b" />);
    caretAt(field(), 0);

    moveCaret(field(), 'right');

    expect(field().selectionStart).toBe(2);
  });

  it('never changes the value', () => {
    render(<Field initial="abc" />);
    caretAt(field(), 1);

    moveCaret(field(), 'right');

    expect(rendered()).toBe('abc');
  });
});

describe('clearField', () => {
  it('empties a controlled field', () => {
    render(<Field initial="abc" />);

    expect(clearField(field())).toBe(true);
    expect(rendered()).toBe('');
  });

  it('reports nothing to do on an empty field', () => {
    render(<Field initial="" />);

    expect(clearField(field())).toBe(false);
  });
});

describe('isEditableField', () => {
  it('accepts textual inputs and textareas', () => {
    render(
      <>
        <input aria-label="t" type="text" />
        <input aria-label="s" type="search" />
        <input aria-label="p" type="password" />
        <textarea aria-label="a" />
      </>,
    );

    for (const label of ['t', 's', 'p', 'a']) {
      expect(isEditableField(screen.getByLabelText(label))).toBe(true);
    }
  });

  it('rejects controls that are not free text', () => {
    render(
      <>
        <input aria-label="n" type="number" />
        <input aria-label="c" type="checkbox" />
        <input aria-label="r" type="range" />
        <select aria-label="sel" />
        <button type="button">b</button>
      </>,
    );

    for (const label of ['n', 'c', 'r', 'sel']) {
      expect(isEditableField(screen.getByLabelText(label))).toBe(false);
    }
    expect(isEditableField(screen.getByText('b'))).toBe(false);
  });

  it('rejects read-only and disabled fields', () => {
    render(
      <>
        <input aria-label="ro" type="text" readOnly />
        <input aria-label="dis" type="text" disabled />
      </>,
    );

    expect(isEditableField(screen.getByLabelText('ro'))).toBe(false);
    expect(isEditableField(screen.getByLabelText('dis'))).toBe(false);
  });

  it('handles null', () => {
    expect(isEditableField(null)).toBe(false);
  });
});
