import { describe, expect, it } from 'vitest';
import { datapackInstallNote, datapackWorldLabel } from './datapackWorlds';
import type { DatapackWorldStatus } from './tauri';

const status = (over: Partial<DatapackWorldStatus>): DatapackWorldStatus => ({
  all_worlds: true,
  selected_worlds: [],
  available_worlds: ['A', 'B', 'C'],
  covered_worlds: 3,
  ...over,
});

describe('datapackWorldLabel', () => {
  it('has nothing to say for rows that are not data packs', () => {
    expect(datapackWorldLabel(undefined, true)).toBeNull();
    expect(datapackWorldLabel(null, true)).toBeNull();
  });

  it('counts worlds when the pack goes to all of them', () => {
    expect(datapackWorldLabel(status({}), true)).toBe('All worlds (3)');
  });

  it('says how many of the worlds a chosen set covers', () => {
    expect(datapackWorldLabel(status({ all_worlds: false, selected_worlds: ['A', 'B'], covered_worlds: 2 }), true)).toBe('2 of 3 worlds');
  });

  it('treats a chosen set that covers every world as all worlds', () => {
    expect(datapackWorldLabel(status({ all_worlds: false, selected_worlds: ['A', 'B', 'C'], covered_worlds: 3 }), true)).toBe('All worlds (3)');
  });

  it('explains the wait when there are no worlds yet', () => {
    expect(datapackWorldLabel(status({ available_worlds: [], covered_worlds: 0 }), true)).toBe(
      'No worlds yet — added to new worlds from their second session',
    );
  });

  it('flags a choice that matches no world', () => {
    expect(datapackWorldLabel(status({ all_worlds: false, selected_worlds: ['Gone'], covered_worlds: 0 }), true)).toBe('No worlds selected');
  });

  it('says a disabled pack is in no world', () => {
    expect(datapackWorldLabel(status({}), false)).toBe('Not in any world while disabled');
  });
});

describe('datapackInstallNote', () => {
  it('states the world count and the new-world caveat', () => {
    expect(datapackInstallNote(3)).toContain('Will be added to all 3 worlds in this instance.');
    expect(datapackInstallNote(1)).toContain('all 1 world in this instance');
    expect(datapackInstallNote(3)).toContain('second session');
  });

  it('handles an instance with no worlds', () => {
    expect(datapackInstallNote(0)).toContain('no worlds yet');
  });
});
