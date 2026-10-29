import { useEffect, useState } from 'react';
import type { WindowSize } from '@/lib/tauri';

const PRESETS: WindowSize[] = [
  { width: 854, height: 480 },
  { width: 1280, height: 720 },
  { width: 1600, height: 900 },
  { width: 1920, height: 1080 },
];

/**
 * Pick a fixed game window size, or none. `noneLabel` names what applies
 * without one ("Use the default from Settings", "Let the game decide").
 */
export function WindowSizeEditor({
  value,
  noneLabel,
  disabled,
  onSave,
}: {
  value: WindowSize | null;
  noneLabel: string;
  disabled?: boolean;
  onSave: (size: WindowSize | null) => Promise<void>;
}) {
  const [fixed, setFixed] = useState(value !== null);
  const [width, setWidth] = useState(String(value?.width ?? 1280));
  const [height, setHeight] = useState(String(value?.height ?? 720));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setFixed(value !== null);
    if (value) {
      setWidth(String(value.width));
      setHeight(String(value.height));
    }
  }, [value]);

  const parsed = { width: Number(width), height: Number(height) };
  const valid = [parsed.width, parsed.height].every((n) => Number.isInteger(n) && n >= 200 && n <= 16384);
  const dirty = fixed
    ? !value || value.width !== parsed.width || value.height !== parsed.height
    : value !== null;

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      await onSave(fixed ? parsed : null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-2">
      <label className="flex items-center gap-2 text-sm">
        <input type="radio" checked={!fixed} disabled={disabled} onChange={() => setFixed(false)} className="accent-primary" />
        {noneLabel}
      </label>
      <label className="flex items-center gap-2 text-sm">
        <input type="radio" checked={fixed} disabled={disabled} onChange={() => setFixed(true)} className="accent-primary" />
        Fixed size
      </label>
      {fixed && (
        <div className="flex flex-wrap items-center gap-2 pl-6">
          <input
            aria-label="Window width"
            type="number"
            min={200}
            max={16384}
            value={width}
            disabled={disabled}
            onChange={(e) => setWidth(e.target.value)}
            className="w-24 rounded-lg border border-input bg-background px-2 py-1 text-sm"
          />
          <span aria-hidden="true">×</span>
          <input
            aria-label="Window height"
            type="number"
            min={200}
            max={16384}
            value={height}
            disabled={disabled}
            onChange={(e) => setHeight(e.target.value)}
            className="w-24 rounded-lg border border-input bg-background px-2 py-1 text-sm"
          />
          {PRESETS.map((preset) => (
            <button
              key={`${preset.width}x${preset.height}`}
              type="button"
              disabled={disabled}
              onClick={() => { setWidth(String(preset.width)); setHeight(String(preset.height)); }}
              className="rounded-md border border-input px-2 py-0.5 text-xs hover:bg-accent disabled:opacity-50"
            >
              {preset.width}×{preset.height}
            </button>
          ))}
        </div>
      )}
      {fixed && !valid && <p className="pl-6 text-xs text-destructive">Width and height must each be between 200 and 16384.</p>}
      {error && <p className="text-xs text-destructive">{error}</p>}
      <button
        type="button"
        onClick={() => void save()}
        disabled={disabled || saving || !dirty || (fixed && !valid)}
        className="rounded-lg border border-input px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50"
      >
        {saving ? 'Saving…' : 'Save window size'}
      </button>
    </div>
  );
}
