/**
 * Opening a named Settings section from elsewhere in the app (the command
 * palette, today).
 *
 * Settings remembers its top-level tab, not its sub-page, and it only reads
 * what it remembers when it mounts. A request is therefore held here until the
 * next Settings mount takes it, and also announced to a Settings that is
 * already open, which takes it on the event.
 */

export interface SettingsSectionTarget {
  /** Settings tab id, e.g. `appearance`. */
  tab: string;
  /** Sub-page id inside that tab, e.g. `interface`. */
  page: string;
}

export const SETTINGS_SECTION_EVENT = 'agora-settings-section';

let pending: SettingsSectionTarget | null = null;

export function requestSettingsSection(target: SettingsSectionTarget): void {
  pending = target;
  window.dispatchEvent(new CustomEvent(SETTINGS_SECTION_EVENT));
}

/** Read the held request without consuming it (a mount's first render). */
export function peekPendingSettingsSection(): SettingsSectionTarget | null {
  return pending;
}

/** Consume the held request, so one request opens one section once. */
export function takePendingSettingsSection(): SettingsSectionTarget | null {
  const target = pending;
  pending = null;
  return target;
}

export interface SettingsSectionEntry {
  label: string;
  icon: string;
  /** Other words that find this entry, besides its label. */
  keywords: string[];
  target: SettingsSectionTarget;
}

/** Settings sections the command palette can open directly. */
export const SETTINGS_SECTION_ENTRIES: SettingsSectionEntry[] = [
  {
    label: 'Appearance',
    icon: '🎨',
    keywords: ['theme', 'colour', 'color', 'dark mode', 'light mode'],
    target: { tab: 'appearance', page: 'theme' },
  },
  {
    label: 'Text size',
    icon: '🔠',
    keywords: ['text scale', 'font size', 'font', 'zoom', 'larger text'],
    target: { tab: 'appearance', page: 'interface' },
  },
];

/** Entries whose label or keywords contain the query. An empty query finds all. */
export function matchSettingsSections(query: string): SettingsSectionEntry[] {
  const needle = query.trim().replace(/\s+/g, ' ').toLowerCase();
  if (!needle) return [...SETTINGS_SECTION_ENTRIES];
  return SETTINGS_SECTION_ENTRIES.filter((entry) =>
    [entry.label, ...entry.keywords].some((word) => word.toLowerCase().includes(needle)),
  );
}
