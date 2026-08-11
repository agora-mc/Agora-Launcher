/**
 * High Interaction presentation preference (versioned, local, safe default).
 *
 * MASTER_ARCHITECTURE §5.2/5.3: High Interaction Mode is a reversible
 * presentation preference, not a data mode. Persist only the preference
 * (`standard` | `high-interaction`), versioned, with a safe default of
 * `standard`. No instance data is stored here.
 */

export type InteractionPreference = 'standard' | 'high-interaction';

export interface InteractionPreferenceRecord {
  version: 1;
  value: InteractionPreference;
}

const STORAGE_KEY = 'agora-interaction-preference';
const VERSION = 1 as const;
const DEFAULT: InteractionPreference = 'standard';

function getStorage(): Storage | null {
  if (typeof window !== 'undefined' && window.localStorage) return window.localStorage;
  return null;
}

export function loadPreference(): InteractionPreference {
  try {
    const storage = getStorage();
    if (!storage) return DEFAULT;
    const raw = storage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT;
    const parsed = JSON.parse(raw) as Partial<InteractionPreferenceRecord> | null;
    if (!parsed || parsed.version !== VERSION) return DEFAULT;
    if (parsed.value === 'standard' || parsed.value === 'high-interaction') return parsed.value;
    return DEFAULT;
  } catch {
    return DEFAULT;
  }
}

export function savePreference(value: InteractionPreference): InteractionPreferenceRecord {
  const record: InteractionPreferenceRecord = { version: VERSION, value };
  try {
    const storage = getStorage();
    if (!storage) return record;
    storage.setItem(STORAGE_KEY, JSON.stringify(record));
  } catch {
    // Preference remains for the current session when storage is unavailable.
  }
  return record;
}

export function clearPreference(): InteractionPreference {
  savePreference(DEFAULT);
  return DEFAULT;
}

/**
 * Session view state, deliberately separate from the persisted preference
 * (SOL §22.4).
 *
 * SOL-2 §18.4 requires every contextual bridge to LEAVE High Interaction before
 * Standard work begins, so the live host unmounts and no stale live state can
 * survive a Standard close/cancel/rejection/failure. That requirement is about
 * the live *session*, not about the user's saved choice — but a single setter
 * did both, so every review permanently reverted the user's preference to
 * `standard` (T6-4).
 *
 * `suspendHighInteraction()` drops the view for the current navigation only.
 * The unmount, the fresh read on re-entry, and the render-time instance-identity
 * guard (§19.4) are all unchanged, so §18.4 still holds verbatim.
 */
let suspended = false;

export function suspendHighInteraction(): void {
  suspended = true;
}

export function resumeHighInteractionView(): void {
  suspended = false;
}

export function isHighInteractionSuspended(): boolean {
  return suspended;
}

/** The view to show right now: the preference unless the session suspended it. */
export function effectiveView(): InteractionPreference {
  if (suspended) return 'standard';
  return loadPreference();
}
