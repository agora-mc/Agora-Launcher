import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  SETTINGS_SECTION_EVENT,
  matchSettingsSections,
  peekPendingSettingsSection,
  requestSettingsSection,
  takePendingSettingsSection,
} from './settingsSection';

afterEach(() => {
  // Drop any request a test left behind.
  takePendingSettingsSection();
});

const labels = (query: string) => matchSettingsSections(query).map((entry) => entry.label);

describe('matchSettingsSections', () => {
  it('finds Appearance and Text size by their labels', () => {
    expect(labels('Appearance')).toEqual(['Appearance']);
    expect(labels('text size')).toEqual(['Text size']);
  });

  it('finds Text size by the words people use for it', () => {
    expect(labels('text scale')).toEqual(['Text size']);
    expect(labels('font size')).toEqual(['Text size']);
  });

  it('ignores case, and surrounding or stray whitespace', () => {
    expect(labels('TEXT SIZE')).toEqual(['Text size']);
    expect(labels('  text   scale  ')).toEqual(['Text size']);
    expect(labels('  Appearance  ')).toEqual(['Appearance']);
  });

  it('an empty or whitespace-only query finds every section', () => {
    expect(labels('')).toEqual(['Appearance', 'Text size']);
    expect(labels('   ')).toEqual(['Appearance', 'Text size']);
  });

  it('finds nothing for a query that matches no section', () => {
    expect(labels('zzz-no-such-setting')).toEqual([]);
  });

  it('each section opens a real Settings tab and page', () => {
    const targets = matchSettingsSections('').map((entry) => entry.target);
    expect(targets).toEqual([
      { tab: 'appearance', page: 'theme' },
      { tab: 'appearance', page: 'interface' },
    ]);
  });
});

describe('pending Settings section request', () => {
  it('peek reads the request without consuming it', () => {
    requestSettingsSection({ tab: 'appearance', page: 'interface' });
    expect(peekPendingSettingsSection()).toEqual({ tab: 'appearance', page: 'interface' });
    expect(peekPendingSettingsSection()).toEqual({ tab: 'appearance', page: 'interface' });
  });

  it('take consumes the request, so it opens one section once', () => {
    requestSettingsSection({ tab: 'appearance', page: 'interface' });
    expect(takePendingSettingsSection()).toEqual({ tab: 'appearance', page: 'interface' });
    expect(takePendingSettingsSection()).toBeNull();
    expect(peekPendingSettingsSection()).toBeNull();
  });

  it('a later request replaces an unconsumed one', () => {
    requestSettingsSection({ tab: 'appearance', page: 'theme' });
    requestSettingsSection({ tab: 'appearance', page: 'interface' });
    expect(takePendingSettingsSection()).toEqual({ tab: 'appearance', page: 'interface' });
  });

  it('taking with nothing requested returns null', () => {
    expect(takePendingSettingsSection()).toBeNull();
  });

  it('announces the request to an open Settings', () => {
    const listener = vi.fn();
    window.addEventListener(SETTINGS_SECTION_EVENT, listener);
    try {
      requestSettingsSection({ tab: 'appearance', page: 'interface' });
      expect(listener).toHaveBeenCalledTimes(1);
    } finally {
      window.removeEventListener(SETTINGS_SECTION_EVENT, listener);
    }
  });
});
