/**
 * Journal storage — moves the Field Guide progress from WebView Local Storage
 * into the Agora Local AppData (`local_state.db` via getSetting/setSetting).
 *
 * Deleting `%LOCALAPPDATA%\agora` (or `AGORA_DATA_DIR`) must reset progress,
 * which was not true while it lived in WebView2's Local Storage
 * (`%APPDATA%\com.agoramc.app\EBWebView\…`).
 *
 * Strategy mirrors `ambienceSettings.ts`: Tauri first, localStorage fallback,
 * one-time migration from localStorage to Tauri.
 */

import { getSetting, setSetting } from '@/lib/tauri';
import { JOURNAL_KEY as EGGS_JOURNAL_KEY } from './engine/eggs';

export const JOURNAL_KEY = EGGS_JOURNAL_KEY;

// In-memory cache so synchronous callers (engine's buildWorld) can read the
// Tauri value without awaiting, once it has been preloaded.
let cached: string | null | undefined = undefined;
let inflight: Promise<string | null> | null = null;

async function readTauri(): Promise<string | null | undefined> {
  try {
    const v = await getSetting(JOURNAL_KEY);
    if (v === null || v === undefined) return null;
    if (typeof v === 'string') return v;
    // Defensive: if stored as non-string JSON, re-stringify
    try {
      return JSON.stringify(v);
    } catch {
      return null;
    }
  } catch {
    // Tauri not available (tests, web preview, non-Tauri dev)
    return undefined;
  }
}

async function writeTauri(raw: string | null): Promise<boolean> {
  try {
    await setSetting(JOURNAL_KEY, raw as unknown as string | null);
    return true;
  } catch {
    return false;
  }
}

/**
 * Load the journal raw JSON. Prefers Tauri (`local_state.db` inside the Agora
 * app data dir). Migrates a legacy localStorage value once, then clears the
 * WebView storage so future deletes of the Agora folder truly reset progress.
 */
export async function loadJournalRaw(): Promise<string | null> {
  if (cached !== undefined) return cached;
  if (inflight) return inflight;
  inflight = (async () => {
    const fromTauri = await readTauri();
    if (fromTauri !== undefined) {
      if (fromTauri !== null) {
        // Tauri owns the value — drop stale WebView copy
        try {
          window.localStorage.removeItem(JOURNAL_KEY);
        } catch {}
        cached = fromTauri;
        return fromTauri;
      }
      // Tauri has no value — check legacy WebView storage for one-time migration
      try {
        const legacy = window.localStorage.getItem(JOURNAL_KEY);
        if (legacy !== null) {
          const ok = await writeTauri(legacy);
          if (ok) {
            try {
              window.localStorage.removeItem(JOURNAL_KEY);
            } catch {}
          }
          cached = legacy;
          return legacy;
        }
      } catch {}
      cached = null;
      return null;
    }
    // No Tauri — fallback to WebView storage (tests, web)
    try {
      const raw = window.localStorage.getItem(JOURNAL_KEY);
      cached = raw;
      return raw;
    } catch {
      cached = null;
      return null;
    }
  })();
  const result = await inflight;
  inflight = null;
  return result;
}

/**
 * Persist the journal raw JSON. Writes to Tauri when available and clears the
 * WebView copy; falls back to localStorage otherwise.
 */
export async function saveJournalRaw(raw: string | null): Promise<void> {
  cached = raw;
  const ok = await writeTauri(raw);
  if (ok) {
    try {
      window.localStorage.removeItem(JOURNAL_KEY);
    } catch {}
    return;
  }
  try {
    if (raw === null) window.localStorage.removeItem(JOURNAL_KEY);
    else window.localStorage.setItem(JOURNAL_KEY, raw);
  } catch {}
}

/** Ensure the in-memory cache is populated (call before creating the engine). */
export async function ensureJournalLoaded(): Promise<string | null> {
  return loadJournalRaw();
}

/** Synchronous read of the cached value, falling back to localStorage for tests/web. */
export function getCachedJournalRawSync(): string | null {
  if (cached !== undefined) return cached;
  try {
    return window.localStorage.getItem(JOURNAL_KEY);
  } catch {
    return null;
  }
}

/** Test helper to reset in-memory cache. */
export function __resetJournalCache(): void {
  cached = undefined;
  inflight = null;
}
