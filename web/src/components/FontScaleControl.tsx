'use client';

import { useEffect, useState } from 'react';

const STORAGE_KEY = 'agora-web-font-scale';
const MIN_SCALE = 0.85;
const MAX_SCALE = 2.0;
const STEP = 0.05;
const DEFAULT_SCALE = 1;

function clamp(value: number) {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, value));
}

export function FontScaleControl() {
  const [scale, setScale] = useState<number>(DEFAULT_SCALE);
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      const parsed = stored ? Number.parseFloat(stored) : DEFAULT_SCALE;
      const next = Number.isFinite(parsed) ? clamp(parsed) : DEFAULT_SCALE;
      setScale(next);
      document.documentElement.style.setProperty('--font-scale', String(next));
    } catch {
      // ignore
    }
    setMounted(true);
  }, []);

  useEffect(() => {
    if (!mounted) return;
    document.documentElement.style.setProperty('--font-scale', String(scale));
    try {
      localStorage.setItem(STORAGE_KEY, String(scale));
    } catch {
      // ignore
    }
  }, [scale, mounted]);

  if (!mounted) {
    return (
      <div className="flex items-center gap-1.5 text-xs text-gray-500" aria-hidden="true">
        <span className="h-7 w-7 rounded border border-gold/20 bg-surface" />
        <span className="h-7 w-16 rounded border border-gold/20 bg-surface" />
        <span className="h-7 w-7 rounded border border-gold/20 bg-surface" />
      </div>
    );
  }

  const percent = Math.round(scale * 100);

  return (
    <div className="flex items-center gap-1.5" role="group" aria-label="Font size">
      <span className="hidden text-xs text-gray-500 dark:text-gray-400 sm:inline">Text size</span>
      <button
        type="button"
        onClick={() => setScale((s) => clamp(Number((s - STEP).toFixed(2))))}
        disabled={scale <= MIN_SCALE}
        aria-label="Decrease font size"
        className="flex h-7 w-7 items-center justify-center rounded border border-gray-300 bg-white text-sm font-bold hover:bg-gray-100 disabled:opacity-40 dark:border-gray-600 dark:bg-gray-700 dark:hover:bg-gray-600"
      >
        −
      </button>
      <span className="min-w-[3.25rem] text-center text-xs font-medium tabular-nums text-gray-700 dark:text-gray-300" aria-live="polite">
        {percent}%
      </span>
      <button
        type="button"
        onClick={() => setScale((s) => clamp(Number((s + STEP).toFixed(2))))}
        disabled={scale >= MAX_SCALE}
        aria-label="Increase font size"
        className="flex h-7 w-7 items-center justify-center rounded border border-gray-300 bg-white text-sm font-bold hover:bg-gray-100 disabled:opacity-40 dark:border-gray-600 dark:bg-gray-700 dark:hover:bg-gray-600"
      >
        +
      </button>
      {scale !== DEFAULT_SCALE && (
        <button
          type="button"
          onClick={() => setScale(DEFAULT_SCALE)}
          className="ml-1 text-xs text-indigo-600 hover:underline dark:text-indigo-400"
        >
          Reset
        </button>
      )}
    </div>
  );
}
