/**
 * Lab progress store: versioned, non-sensitive, local-only.
 *
 * Sol-0 contract: `docs/interactive/MASTER_ARCHITECTURE.md` §5.3. Progress
 * records adventure ID, lesson version, completed checkpoints, last safe
 * stage, and completion time. It must not contain instance IDs, paths, logs,
 * installed-content names, account data, or live settings. A schema mismatch
 * resets safely rather than attempting a lossy migration.
 *
 * Lab code must not import Tauri, `lib/tauri`, `live/`, or current operation
 * components — enforced by `scripts/check-interactive-boundaries.mjs`.
 */

export interface AdventureProgress {
  /** Authoritative lesson version for which this record is valid. */
  lessonVersion: number;
  completedCheckpoints: number;
  lastSafeCheckpoint: number;
  completed: boolean;
  completedAt?: string;
}

export interface LabProgressRecord {
  version: 1;
  adventures: Record<string, AdventureProgress>;
}

const STORAGE_KEY = 'agora-lab-progress';
const VERSION = 1 as const;

/**
 * Storage accessor. Uses `window.localStorage` explicitly so the browser and
 * jsdom agree; some Node runtimes expose a global `localStorage` that lacks
 * the full Storage API and must not be used.
 */
function getStorage(): Storage | null {
  if (typeof window !== 'undefined' && window.localStorage) {
    return window.localStorage;
  }
  return null;
}

export function emptyProgress(): LabProgressRecord {
  return { version: VERSION, adventures: {} };
}

function isAdventureProgress(value: unknown): value is AdventureProgress {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.lessonVersion === 'number'
    && typeof record.completedCheckpoints === 'number'
    && typeof record.lastSafeCheckpoint === 'number'
    && typeof record.completed === 'boolean'
    && (record.completedAt === undefined || typeof record.completedAt === 'string')
  );
}

export function loadProgress(): LabProgressRecord {
  try {
    const storage = getStorage();
    if (!storage) return emptyProgress();
    const raw = storage.getItem(STORAGE_KEY);
    if (!raw) return emptyProgress();
    const parsed = JSON.parse(raw) as Partial<LabProgressRecord> | null;
    if (!parsed || parsed.version !== VERSION || !parsed.adventures || typeof parsed.adventures !== 'object') {
      return emptyProgress();
    }
    const adventures: Record<string, AdventureProgress> = {};
    for (const [id, value] of Object.entries(parsed.adventures)) {
      if (isAdventureProgress(value)) adventures[id] = value;
    }
    return { version: VERSION, adventures };
  } catch {
    return emptyProgress();
  }
}

function persist(record: LabProgressRecord) {
  try {
    const storage = getStorage();
    if (!storage) return;
    storage.setItem(STORAGE_KEY, JSON.stringify(record));
  } catch {
    // Progress remains usable for the current session when storage fails.
  }
}

/**
 * Load progress for one adventure, returning the safe resume checkpoint.
 * A stored record whose lesson version does not match `scenarioVersion` is
 * treated as empty (reset safely, no lossy migration).
 */
export function loadAdventureProgress(scenarioId: string, scenarioVersion: number): AdventureProgress | null {
  const stored = loadProgress().adventures[scenarioId];
  if (!stored) return null;
  if (stored.lessonVersion !== scenarioVersion) return null;
  return stored;
}

/** Record one completed decision checkpoint. Returns the updated record. */
export function recordCheckpoint(
  scenarioId: string,
  scenarioVersion: number,
  completedCheckpoints: number,
  lastSafeCheckpoint: number,
  completed: boolean,
): LabProgressRecord {
  const current = loadProgress();
  const previous = loadAdventureProgress(scenarioId, scenarioVersion);
  const next: AdventureProgress = {
    lessonVersion: scenarioVersion,
    completedCheckpoints: Math.max(completedCheckpoints, previous?.completedCheckpoints ?? 0),
    lastSafeCheckpoint: Math.max(lastSafeCheckpoint, previous?.lastSafeCheckpoint ?? 0),
    completed: completed || (previous?.completed ?? false),
    completedAt: completed ? new Date().toISOString() : previous?.completedAt,
  };
  const record: LabProgressRecord = {
    version: VERSION,
    adventures: { ...current.adventures, [scenarioId]: next },
  };
  persist(record);
  return record;
}

export function clearProgress(): LabProgressRecord {
  const record = emptyProgress();
  persist(record);
  return record;
}
