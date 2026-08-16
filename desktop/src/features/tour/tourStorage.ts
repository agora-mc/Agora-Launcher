/**
 * Tour progress, persisted locally and versioned.
 *
 * The walkthrough is a per-machine UI aid, so it lives in `localStorage`
 * alongside the shell layout and the presentation preference rather than in
 * the settings database. An unreadable or unknown record degrades to "never
 * run" instead of throwing, and storage being unavailable only costs the
 * resume-after-restart behaviour.
 */

import type { TourState, TourStatus } from './tourModel';

const STORAGE_KEY = 'agora-tour';
const VERSION = 1 as const;

export interface TourRecord {
  version: 1;
  status: TourStatus;
  index: number;
  /** Set once the user reaches the end, so entry points can offer a replay. */
  completed: boolean;
}

export const DEFAULT_TOUR_RECORD: TourRecord = {
  version: VERSION,
  status: 'idle',
  index: 0,
  completed: false,
};

function getStorage(): Storage | null {
  try {
    if (typeof window !== 'undefined' && window.localStorage) return window.localStorage;
  } catch {
    // Storage access can throw outright under some privacy settings.
  }
  return null;
}

function isStatus(value: unknown): value is TourStatus {
  return value === 'idle' || value === 'running' || value === 'finished';
}

export function loadTourRecord(): TourRecord {
  try {
    const storage = getStorage();
    if (!storage) return DEFAULT_TOUR_RECORD;
    const raw = storage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_TOUR_RECORD;
    const parsed = JSON.parse(raw) as Partial<TourRecord> | null;
    if (!parsed || parsed.version !== VERSION) return DEFAULT_TOUR_RECORD;
    if (!isStatus(parsed.status)) return DEFAULT_TOUR_RECORD;
    const index = typeof parsed.index === 'number' && Number.isFinite(parsed.index)
      ? Math.max(0, Math.floor(parsed.index))
      : 0;
    return {
      version: VERSION,
      status: parsed.status,
      index,
      completed: parsed.completed === true,
    };
  } catch {
    return DEFAULT_TOUR_RECORD;
  }
}

export function saveTourRecord(state: TourState, completed: boolean): TourRecord {
  const record: TourRecord = {
    version: VERSION,
    status: state.status,
    index: state.index,
    completed,
  };
  try {
    getStorage()?.setItem(STORAGE_KEY, JSON.stringify(record));
  } catch {
    // Progress still holds for the current session when storage is unavailable.
  }
  return record;
}
