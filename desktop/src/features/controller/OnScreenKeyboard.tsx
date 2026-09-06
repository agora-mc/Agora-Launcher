/**
 * A basic-Latin fallback for controller text entry. It deliberately has no
 * IME, composition, dead-key, autocomplete, or platform selection model; platform text
 * input adapters can take over at the same defaultAccept call site later.
 *
 * The field ref is intentional. Focusing a key moves DOM focus away from the
 * input, and looking at document.activeElement here would make the keyboard
 * edit its own buttons instead of the field that opened it.
 */
import { forwardRef, useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import {
  clearField,
  deleteBackwards,
  insertText,
  isEditableField,
  moveCaret,
  type EditableField,
} from './textEditing';
import { CONTROLLER_LAYER_OVERLAY, useControllerLayer } from './useControllerLayer';

type KeyboardMode = 'lower' | 'upper' | 'symbols';

const LETTER_ROWS = ['qwertyuiop', 'asdfghjkl', 'zxcvbnm'];
const SYMBOL_ROWS = [
  ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'],
  ['-', '_', '/', '\\', ':', ';', '(', ')', '$', '&'],
  ['@', '#', '!', '?', '%', '*', '+', '=', '.', ','],
];

export interface OnScreenKeyboardProps {
  field: EditableField;
  onClose: () => void;
}

interface KeyButtonProps {
  ariaLabel: string;
  children: ReactNode;
  className?: string;
  onClick: () => void;
}

const KeyButton = forwardRef<HTMLButtonElement, KeyButtonProps>(function KeyButton(
  { ariaLabel, children, className = '', onClick },
  ref,
) {
  return (
    <button
      ref={ref}
      type="button"
      aria-label={ariaLabel}
      onClick={onClick}
      className={`flex min-h-14 min-w-0 flex-1 items-center justify-center rounded-xl border border-border bg-background px-2 text-lg font-semibold text-foreground shadow-sm transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring active:bg-primary active:text-primary-foreground ${className}`}
    >
      {children}
    </button>
  );
});

export function OnScreenKeyboard({ field, onClose }: OnScreenKeyboardProps) {
  const rootRef = useRef<HTMLElement>(null);
  const firstKeyRef = useRef<HTMLButtonElement>(null);
  const fieldRef = useRef<EditableField | null>(field);
  const [mode, setMode] = useState<KeyboardMode>('lower');

  const editField = useCallback((operation: (target: EditableField) => boolean) => {
    const target = fieldRef.current;
    if (!target || !target.isConnected || !isEditableField(target)) {
      onClose();
      return;
    }
    operation(target);
  }, [onClose]);

  useControllerLayer({
    rootRef,
    priority: CONTROLLER_LAYER_OVERLAY,
    onCancel: onClose,
  });

  useEffect(() => {
    const target = fieldRef.current;
    if (!target?.isConnected || !isEditableField(target)) {
      onClose();
      return undefined;
    }
    firstKeyRef.current?.focus({ preventScroll: true });

    const observer = new MutationObserver(() => {
      if (!fieldRef.current?.isConnected || !isEditableField(fieldRef.current)) onClose();
    });
    observer.observe(document.body, {
      attributes: true,
      attributeFilter: ['disabled', 'readonly'],
      childList: true,
      subtree: true,
    });
    return () => observer.disconnect();
  }, [onClose]);

  useEffect(() => {
    const handlePhysicalKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing || event.ctrlKey || event.metaKey || event.altKey) return;

      if (event.key === 'Escape' || event.key === 'Enter') {
        event.preventDefault();
        onClose();
        return;
      }
      if (event.key === 'Backspace') {
        event.preventDefault();
        editField(deleteBackwards);
        return;
      }
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
        event.preventDefault();
        editField((target) => moveCaret(target, event.key === 'ArrowLeft' ? 'left' : 'right'));
        return;
      }
      if (event.key.length === 1) {
        event.preventDefault();
        editField((target) => insertText(target, event.key));
      }
    };

    window.addEventListener('keydown', handlePhysicalKeyDown);
    return () => window.removeEventListener('keydown', handlePhysicalKeyDown);
  }, [editField, onClose]);

  const labels = mode === 'symbols'
    ? SYMBOL_ROWS
    : LETTER_ROWS.map((row) => Array.from(mode === 'upper' ? row.toUpperCase() : row));

  const toggleSymbols = () => setMode((current) => (current === 'symbols' ? 'lower' : 'symbols'));
  const toggleShift = () => setMode((current) => {
    if (current === 'symbols') return 'upper';
    return current === 'upper' ? 'lower' : 'upper';
  });

  return createPortal(
    <div
      className="fixed inset-0 z-[90] flex items-end justify-center bg-background/45 p-3 backdrop-blur-sm sm:p-6"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <section
        ref={rootRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="on-screen-keyboard-title"
        className="w-full max-w-5xl rounded-2xl border border-border bg-card/95 p-3 text-card-foreground shadow-2xl sm:p-5"
      >
        <div className="mb-3 flex items-center justify-between gap-3 px-1">
          <div>
            <h2 id="on-screen-keyboard-title" className="text-base font-bold sm:text-lg">On-screen keyboard</h2>
            <p className="text-xs text-muted-foreground sm:text-sm">Editing the focused text field</p>
          </div>
          <span className="rounded-lg border border-border px-2 py-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">B: close</span>
        </div>

        <div className="space-y-2" aria-label="Keyboard keys">
          {labels.map((row, rowIndex) => (
            <div
              key={rowIndex}
              className={`flex gap-2 ${rowIndex === 1 ? 'px-[4%]' : ''} ${rowIndex === 2 ? 'px-[10%]' : ''}`}
            >
              {row.map((label, keyIndex) => (
                <KeyButton
                  key={`${label}-${keyIndex}`}
                  ref={rowIndex === 0 && keyIndex === 0 ? firstKeyRef : undefined}
                  ariaLabel={`Type ${label}`}
                  onClick={() => editField((target) => insertText(target, label))}
                >
                  {label}
                </KeyButton>
              ))}
            </div>
          ))}
        </div>

        <div className="mt-3 flex flex-wrap gap-2">
          <KeyButton
            ariaLabel={mode === 'symbols' ? 'Show letters' : 'Show numbers and symbols'}
            className="min-w-20 px-3 text-sm sm:text-base"
            onClick={toggleSymbols}
          >
            {mode === 'symbols' ? 'ABC' : '123'}
          </KeyButton>
          <KeyButton
            ariaLabel="Shift"
            className={`min-w-20 px-3 text-sm sm:text-base ${mode === 'upper' ? 'bg-primary text-primary-foreground' : ''}`}
            onClick={toggleShift}
          >
            Shift
          </KeyButton>
          <KeyButton
            ariaLabel="Space"
            className="min-w-28 flex-[3] px-3 text-sm sm:text-base"
            onClick={() => editField((target) => insertText(target, ' '))}
          >
            Space
          </KeyButton>
          <KeyButton
            ariaLabel="Move caret left"
            className="min-w-14 px-3 text-2xl"
            onClick={() => editField((target) => moveCaret(target, 'left'))}
          >
            ←
          </KeyButton>
          <KeyButton
            ariaLabel="Move caret right"
            className="min-w-14 px-3 text-2xl"
            onClick={() => editField((target) => moveCaret(target, 'right'))}
          >
            →
          </KeyButton>
          <KeyButton
            ariaLabel="Backspace"
            className="min-w-24 px-3 text-sm sm:text-base"
            onClick={() => editField(deleteBackwards)}
          >
            ⌫ Backspace
          </KeyButton>
          <KeyButton
            ariaLabel="Clear field"
            className="min-w-20 px-3 text-sm sm:text-base"
            onClick={() => editField(clearField)}
          >
            Clear
          </KeyButton>
          <KeyButton
            ariaLabel="Done"
            className="min-w-20 bg-primary px-3 text-sm text-primary-foreground hover:bg-primary/90 sm:text-base"
            onClick={onClose}
          >
            Done
          </KeyButton>
        </div>
      </section>
    </div>,
    document.body,
  );
}
