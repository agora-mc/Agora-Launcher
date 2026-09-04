import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check } from 'lucide-react';
import {
  commitSelectValue,
  selectChoices,
  selectedChoiceIndex,
  type SelectChoice,
} from './elementAdapters';
import { CONTROLLER_LAYER_OVERLAY, useControllerLayer } from './useControllerLayer';

export interface SelectOverlayProps {
  select: HTMLSelectElement;
  onClose: () => void;
}

/**
 * An in-app replacement for a native dropdown's system popup.
 *
 * The popup a `<select>` opens is drawn by the operating system, outside the
 * page, where the Gamepad API cannot see or steer it. Rather than rewrite every
 * dropdown in the app onto a custom listbox, the native element stays exactly
 * where it is and this takes over only the moment of choosing: Accept opens
 * this, the stick moves through the options, Accept commits and closes, Cancel
 * leaves the value alone.
 *
 * The options are real buttons in a layer, so the spatial navigation and focus
 * ring already built move between them without any list-navigation code here.
 *
 * The value is written through the same native setter everything else uses,
 * because assigning `.value` would update the DOM while skipping React's
 * onChange and the setting would snap back on the next render.
 */
export function SelectOverlay({ select, onClose }: SelectOverlayProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const selectRef = useRef(select);
  const [choices] = useState<SelectChoice[]>(() => selectChoices(select));
  const initialIndex = useRef(selectedChoiceIndex(select));
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);

  useControllerLayer({
    rootRef,
    priority: CONTROLLER_LAYER_OVERLAY,
    onCancel: onClose,
  });

  // Start on the current value, so Accept twice is a no-op rather than a
  // silent change to whatever happened to be first in the list.
  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      const target = optionRefs.current[initialIndex.current] ?? optionRefs.current[0];
      target?.focus({ preventScroll: true });
      target?.scrollIntoView?.({ block: 'nearest' });
    });
    return () => cancelAnimationFrame(frame);
  }, []);

  // A re-render can replace the element underneath us; committing to a detached
  // node would look like the choice simply did nothing.
  useEffect(() => {
    if (!selectRef.current.isConnected) onClose();
  });

  const choose = useCallback((value: string) => {
    const target = selectRef.current;
    if (target.isConnected) commitSelectValue(target, value);
    onClose();
  }, [onClose]);

  const label = select.getAttribute('aria-label')
    ?? select.labels?.[0]?.textContent?.trim()
    ?? 'Choose an option';

  if (typeof document === 'undefined') return null;

  return createPortal(
    <div
      className="fixed inset-0 z-[90] flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm"
      // controller-exempt: backdrop, not a control; Cancel closes this overlay.
      onClick={(event) => { if (event.target === event.currentTarget) onClose(); }}
    >
      <div
        ref={rootRef}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        className="flex max-h-[80vh] w-full max-w-md flex-col overflow-hidden rounded-2xl border border-border bg-card shadow-2xl"
      >
        <p className="shrink-0 border-b border-border px-5 py-4 text-sm font-semibold text-card-foreground">
          {label}
        </p>
        <div className="min-h-0 flex-1 overflow-y-auto p-2" role="group">
          {choices.length === 0 ? (
            <p className="px-3 py-6 text-center text-sm text-muted-foreground">Nothing to choose from.</p>
          ) : choices.map((choice, index) => {
            const current = index === initialIndex.current;
            return (
              <button
                key={`${choice.value}-${choice.index}`}
                ref={(element) => { optionRefs.current[index] = element; }}
                type="button"
                aria-pressed={current}
                onClick={() => choose(choice.value)}
                className={`flex w-full items-center justify-between gap-3 rounded-xl px-4 py-3 text-left text-sm transition-colors ${
                  current ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/60'
                }`}
              >
                <span className="min-w-0 truncate">{choice.label}</span>
                {current && <Check className="h-4 w-4 shrink-0" aria-hidden="true" />}
              </button>
            );
          })}
        </div>
        <p className="shrink-0 border-t border-border px-5 py-3 text-xs text-muted-foreground">
          Choose with the stick · A to select · B to cancel
        </p>
      </div>
    </div>,
    document.body,
  );
}
