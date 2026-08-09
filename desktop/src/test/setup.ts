import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

afterEach(() => {
  cleanup();
});

// jsdom does not implement matchMedia; the interactive visuals use it for
// reduced-motion detection (which defaults to the data-motion attribute).
if (typeof window !== 'undefined' && typeof window.matchMedia !== 'function') {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: (query: string): MediaQueryList =>
      ({
        matches: false,
        media: query,
        onchange: null,
        addListener: () => undefined,
        removeListener: () => undefined,
        addEventListener: () => undefined,
        removeEventListener: () => undefined,
        dispatchEvent: () => false,
      }) as unknown as MediaQueryList,
  });
}

/**
 * Node 25 ships a global `localStorage` (webstorage) that shadows jsdom's and
 * is non-functional without `--localstorage-file`. Install a working in-memory
 * implementation on both `window` and `globalThis` so unit tests (and the
 * Lab progress store, which uses `window.localStorage`) behave like a browser.
 */
function installMemoryStorage() {
  const make = (): Storage => {
    const store = new Map<string, string>();
    return {
      get length() {
        return store.size;
      },
      clear: () => store.clear(),
      getItem: (key: string) => (store.has(String(key)) ? store.get(String(key)) ?? null : null),
      key: (index: number) => Array.from(store.keys())[index] ?? null,
      removeItem: (key: string) => void store.delete(String(key)),
      setItem: (key: string, value: string) => void store.set(String(key), String(value)),
    } as unknown as Storage;
  };

  const storage = make();
  const install = (target: object, name: string) => {
    try {
      Object.defineProperty(target, name, { value: storage, configurable: true, writable: true });
    } catch {
      // Non-configurable on this runtime; fall back to assignment where possible.
      (target as Record<string, unknown>)[name] = storage;
    }
  };

  if (typeof window !== 'undefined') install(window, 'localStorage');
  install(globalThis, 'localStorage');
}

installMemoryStorage();
