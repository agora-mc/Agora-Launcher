/**
 * Interaction achievements — the WorldEditor's toast achievements (Searcher,
 * Curious, Rearrange, …). These are now PERSISTED (localStorage) so each is
 * earned once, and the Field Guide page displays them alongside the ambience
 * milestones. The interactive layer stays pure (no ambience, no tauri); this
 * module is just a localStorage-backed store.
 */

export interface InteractionAchievement {
  key: string;
  icon: string;
  name: string;
  detail: string;
}

export const INTERACTION_ACHIEVEMENTS: InteractionAchievement[] = [
  { key: 'searcher', icon: '🔎', name: 'Searcher', detail: 'Used the search box' },
  { key: 'curious', icon: '🔍', name: 'Curious', detail: 'Looked closer at something' },
  { key: 'rearrange', icon: '🧩', name: 'Rearranger', detail: 'Rearranged the shelf' },
  { key: 'tidied-up', icon: '🧹', name: 'Tidy', detail: 'Removed something from the world' },
  { key: 'second-thoughts', icon: '↩️', name: 'Second Thoughts', detail: 'Put something back' },
  { key: 'sorted-it-out', icon: '🗂️', name: 'Organiser', detail: 'Filtered the shelf' },
  { key: 'all-clear', icon: '✅', name: 'All Clear', detail: 'Pre-flight check passed' },
  { key: 'called-doctor', icon: '🩺', name: 'Diagnostician', detail: 'Opened the Crash Doctor' },
  { key: 'lets-go', icon: '🎮', name: 'Let\'s Go', detail: 'Pressed the big button' },
  { key: 'suspect', icon: '🕵️', name: 'Detective', detail: 'Picked a crash suspect' },
];

export const INTERACTION_ACHIEVEMENT_KEY = 'agora-interaction-achievements';

/** Load the earned interaction-achievement key set (best effort). */
export function loadEarnedInteraction(): Set<string> {
  try {
    const raw = window.localStorage.getItem(INTERACTION_ACHIEVEMENT_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return new Set();
    return new Set(parsed.filter((k): k is string => typeof k === 'string'));
  } catch {
    return new Set();
  }
}

/**
 * Try to earn an interaction achievement. Returns true only the FIRST time
 * (and persists it); subsequent calls return false so the toast never repeats
 * (previously every action re-announced the same achievement).
 */
export function tryEarnInteraction(key: string): boolean {
  const earned = loadEarnedInteraction();
  if (earned.has(key)) return false;
  earned.add(key);
  try {
    window.localStorage.setItem(INTERACTION_ACHIEVEMENT_KEY, JSON.stringify([...earned]));
  } catch {
    // best effort
  }
  return true;
}

/** Reset all interaction achievements (dev/test helper). */
export function resetInteractionAchievements(): void {
  try {
    window.localStorage.removeItem(INTERACTION_ACHIEVEMENT_KEY);
  } catch {
    // best effort
  }
}
