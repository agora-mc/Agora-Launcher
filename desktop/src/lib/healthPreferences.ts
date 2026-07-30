import {
  getSetting,
  setSetting,
  type HealthWarning,
} from './tauri';

const HEALTH_PREFERENCES_KEY = 'health_preferences';

export interface HealthPreferences {
  mutedWarnings: string[];
  muteAllRecommendations: boolean;
}

const DEFAULT_PREFERENCES: HealthPreferences = {
  mutedWarnings: [],
  muteAllRecommendations: false,
};

function legacySilenceKey(item: HealthWarning): string {
  return `health_silenced_${item.kind}_${item.mod_id ?? 'global'}`;
}

function stableTextHash(value: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, '0');
}

export function healthWarningKey(item: HealthWarning): string {
  const identity = item.mod_id ?? item.filename ?? 'global';
  return `${item.kind}:${identity}:${stableTextHash(item.message)}`;
}

function isTrue(value: unknown): boolean {
  return value === true || value === 1 || value === '1' || value === 'true';
}

function parsePreferences(value: unknown): HealthPreferences | null {
  if (!value || typeof value !== 'object') return null;
  const raw = value as Record<string, unknown>;
  return {
    mutedWarnings: Array.isArray(raw.mutedWarnings)
      ? raw.mutedWarnings.filter((item): item is string => typeof item === 'string')
      : [],
    muteAllRecommendations: isTrue(raw.muteAllRecommendations),
  };
}

export async function loadHealthPreferences(
  currentWarnings: HealthWarning[],
): Promise<HealthPreferences> {
  const stored = parsePreferences(await getSetting(HEALTH_PREFERENCES_KEY));
  if (stored) return stored;

  const migrated = new Set<string>();
  for (const warning of currentWarnings) {
    if (isTrue(await getSetting(legacySilenceKey(warning)))) {
      migrated.add(healthWarningKey(warning));
    }
  }

  const preferences = {
    ...DEFAULT_PREFERENCES,
    mutedWarnings: [...migrated],
  };
  if (migrated.size > 0) {
    await setSetting(HEALTH_PREFERENCES_KEY, preferences);
  }
  return preferences;
}

export function saveHealthPreferences(preferences: HealthPreferences): Promise<void> {
  return setSetting(HEALTH_PREFERENCES_KEY, preferences);
}

export function activeHealthWarnings(
  warnings: HealthWarning[],
  preferences: HealthPreferences,
): HealthWarning[] {
  const muted = new Set(preferences.mutedWarnings);
  return warnings.filter((warning) => !muted.has(healthWarningKey(warning)));
}
