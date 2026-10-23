import { describe, expect, it } from 'vitest';
import { isTextEntryTarget } from './keyboardTargets';

function input(type?: string) {
  const el = document.createElement('input');
  if (type) el.type = type;
  return el;
}

describe('isTextEntryTarget', () => {
  it('blocks text-entry fields', () => {
    for (const type of [undefined, 'text', 'search', 'password', 'number', 'email']) {
      expect(isTextEntryTarget(input(type))).toBe(true);
    }
    expect(isTextEntryTarget(document.createElement('textarea'))).toBe(true);
    const box = document.createElement('div');
    box.setAttribute('role', 'textbox');
    expect(isTextEntryTarget(box)).toBe(true);
  });

  it('lets sliders, checkboxes and radios through', () => {
    for (const type of ['range', 'checkbox', 'radio', 'button', 'color']) {
      expect(isTextEntryTarget(input(type))).toBe(false);
    }
    expect(isTextEntryTarget(document.createElement('button'))).toBe(false);
    expect(isTextEntryTarget(null)).toBe(false);
  });
});
