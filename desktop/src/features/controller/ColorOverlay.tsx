import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check } from 'lucide-react';
import { setNativeValue } from './elementAdapters';
import { CONTROLLER_LAYER_OVERLAY, useControllerLayer } from './useControllerLayer';

export interface ColorOverlayProps {
  input: HTMLInputElement;
  onClose: () => void;
}

/**
 * A controller-operable colour picker.
 *
 * `<input type="color">` opens an operating-system colour dialog. Like the
 * dropdown popup, it is drawn outside the page and the Gamepad API cannot see
 * or steer it — so on a handheld these seven theme settings were simply not
 * adjustable. Accept opens this instead and the native input never gets the
 * chance to summon its own window.
 *
 * Swatches are buttons and the channels are range inputs, which means the
 * spatial navigation, focus ring and slider adapter already built do all the
 * work; nothing here re-implements movement.
 */

/** Enough coverage to pick a theme by eye, without becoming a colour wheel. */
const PRESETS = [
  '#0f172a', '#1e293b', '#334155', '#64748b', '#94a3b8', '#e2e8f0', '#f8fafc', '#ffffff',
  '#7f1d1d', '#b91c1c', '#ea580c', '#d97706', '#ca8a04', '#65a30d', '#16a34a', '#059669',
  '#0d9488', '#0891b2', '#0284c7', '#2563eb', '#4f46e5', '#7c3aed', '#a21caf', '#be185d',
];

const CHANNELS = [
  { key: 'r', label: 'Red' },
  { key: 'g', label: 'Green' },
  { key: 'b', label: 'Blue' },
] as const;

function clampByte(value: number): number {
  return Math.max(0, Math.min(255, Math.round(value)));
}

function toRgb(hex: string): { r: number; g: number; b: number } {
  const match = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!match) return { r: 0, g: 0, b: 0 };
  const value = Number.parseInt(match[1], 16);
  return { r: (value >> 16) & 255, g: (value >> 8) & 255, b: value & 255 };
}

function toHex({ r, g, b }: { r: number; g: number; b: number }): string {
  const part = (n: number) => clampByte(n).toString(16).padStart(2, '0');
  return `#${part(r)}${part(g)}${part(b)}`;
}

export function ColorOverlay({ input, onClose }: ColorOverlayProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef(input);
  const originalRef = useRef(input.value);
  const [colour, setColour] = useState(() => toRgb(input.value));

  useControllerLayer({
    rootRef,
    priority: CONTROLLER_LAYER_OVERLAY,
    // Cancel means "leave it as it was", so the live preview is undone.
    onCancel: () => {
      commit(originalRef.current);
      onClose();
    },
  });

  const commit = useCallback((hex: string) => {
    const target = inputRef.current;
    if (!target.isConnected) return;
    setNativeValue(target, HTMLInputElement.prototype, hex, ['input', 'change']);
  }, []);

  // Preview as the user moves, so a slider means something without committing
  // to it first. Cancel puts the original back.
  useEffect(() => {
    commit(toHex(colour));
  }, [colour, commit]);

  // A re-render can replace the input; writing to a detached node would look
  // like the picker silently did nothing.
  useEffect(() => {
    if (!inputRef.current.isConnected) onClose();
  });

  const label = input.getAttribute('aria-label')
    ?? input.labels?.[0]?.textContent?.trim()
    ?? 'Choose a colour';
  const hex = toHex(colour);

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
        className="flex max-h-[85vh] w-[min(30rem,calc(100vw-2rem))] flex-col overflow-hidden rounded-2xl border border-border bg-card shadow-2xl"
      >
        <div className="flex shrink-0 items-center gap-3 border-b border-border px-5 py-4">
          <span
            aria-hidden="true"
            className="h-9 w-9 shrink-0 rounded-lg border border-border"
            style={{ backgroundColor: hex }}
          />
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-card-foreground">{label}</p>
            <p className="font-mono text-xs uppercase text-muted-foreground">{hex}</p>
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          <div className="grid grid-cols-8 gap-2">
            {PRESETS.map((preset) => {
              const current = preset.toLowerCase() === hex.toLowerCase();
              return (
                <button
                  key={preset}
                  type="button"
                  aria-label={preset}
                  aria-pressed={current}
                  onClick={() => setColour(toRgb(preset))}
                  className="flex aspect-square items-center justify-center rounded-lg border border-border"
                  style={{ backgroundColor: preset }}
                >
                  {current && <Check className="h-4 w-4 text-white mix-blend-difference" aria-hidden="true" />}
                </button>
              );
            })}
          </div>

          <div className="mt-5 space-y-3">
            {CHANNELS.map(({ key, label: channelLabel }) => (
              <label key={key} className="block space-y-1 text-xs">
                <span className="flex justify-between">
                  <span>{channelLabel}</span>
                  <span className="font-mono">{colour[key]}</span>
                </span>
                <input
                  type="range"
                  aria-label={channelLabel}
                  min="0"
                  max="255"
                  step="1"
                  value={colour[key]}
                  onChange={(event) => setColour((current) => ({
                    ...current,
                    [key]: clampByte(Number(event.target.value)),
                  }))}
                  className="w-full accent-primary"
                />
              </label>
            ))}
          </div>
        </div>

        <div className="flex shrink-0 items-center justify-between gap-3 border-t border-border px-5 py-3">
          <p className="text-xs text-muted-foreground">Left/right on a slider · A to pick · B to cancel</p>
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-semibold text-primary-foreground hover:bg-primary/90"
          >
            Done
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
