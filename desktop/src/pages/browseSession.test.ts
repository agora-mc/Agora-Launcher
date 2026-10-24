import { beforeEach, describe, expect, it } from 'vitest';
import {
  browseResultsStale,
  clearBrowseSnapshot,
  defaultBrowseContextInstance,
  parkBrowseSnapshot,
  peekBrowseContextChoice,
  peekParkedBrowseFilter,
  pickBrowseContextInstance,
  pickDefaultPackRelease,
  saveBrowseContextChoice,
  saveBrowseSnapshot,
  shouldOfferModrinthSearch,
} from './browseSession';

const releases = [
  { version: '3.0', minecraft_version: '26.1.2', loader: 'fabric' },
  { version: '2.5', minecraft_version: '26.2', loader: 'quilt' },
  { version: '2.4', minecraft_version: '26.2', loader: 'fabric' },
];

describe('pickDefaultPackRelease', () => {
  it('keeps the newest release without a filter', () => {
    expect(pickDefaultPackRelease(releases, null)).toEqual({ index: 0, mismatch: false });
  });

  it('prefers the newest release matching the Minecraft version and loader', () => {
    expect(pickDefaultPackRelease(releases, { mcVersion: '26.2', loader: 'Fabric' })).toEqual({
      index: 2,
      mismatch: false,
    });
  });

  it('falls back to a Minecraft-version match when no release has the loader', () => {
    expect(pickDefaultPackRelease(releases, { mcVersion: '26.2', loader: 'forge' })).toEqual({
      index: 1,
      mismatch: false,
    });
  });

  it('keeps the default and flags a mismatch when nothing matches', () => {
    expect(pickDefaultPackRelease(releases, { mcVersion: '1.20.1', loader: null })).toEqual({
      index: 0,
      mismatch: true,
    });
  });
});

describe('peekParkedBrowseFilter', () => {
  beforeEach(() => clearBrowseSnapshot());

  const snapshot = (mcVersion: string | null, loader: string | null) => ({
    queryKey: JSON.stringify({ sort: 'x', category: null, contentType: 'pack', mcVersion, loader, query: '' }),
    items: [],
    hasMore: false,
    currentPage: 0,
    scrollTop: 0,
    bazaarMode: false,
    bazaarSettledOrder: [],
  });

  it('is null unless the user navigated from Browse', () => {
    saveBrowseSnapshot(snapshot('26.2', 'fabric'));
    expect(peekParkedBrowseFilter()).toBeNull();
  });

  it('reads the filters of the parked Browse list', () => {
    saveBrowseSnapshot(snapshot('26.2', 'fabric'));
    parkBrowseSnapshot(0);
    expect(peekParkedBrowseFilter()).toEqual({ mcVersion: '26.2', loader: 'fabric' });
  });

  it('is null when Browse had no version or loader filter', () => {
    saveBrowseSnapshot(snapshot(null, null));
    parkBrowseSnapshot(0);
    expect(peekParkedBrowseFilter()).toBeNull();
  });
});

function row(instance_id: string, last_launched_at: string | null, created_at: string) {
  return { instance_id, last_launched_at, created_at };
}

const OLD = '2026-01-01T00:00:00Z';
const MID = '2026-03-01T00:00:00Z';
const NEW = '2026-06-01T00:00:00Z';

describe('defaultBrowseContextInstance', () => {
  it('is null with no instances', () => {
    expect(defaultBrowseContextInstance([])).toBeNull();
  });

  it('prefers the most recently launched instance', () => {
    const rows = [row('a', OLD, NEW), row('b', NEW, OLD), row('c', MID, OLD)];
    expect(defaultBrowseContextInstance(rows)).toBe('b');
  });

  it('ignores never-launched instances when one was launched', () => {
    const rows = [row('never-new', null, NEW), row('launched-old', OLD, OLD)];
    expect(defaultBrowseContextInstance(rows)).toBe('launched-old');
  });

  it('falls back to the most recently created when nothing was launched', () => {
    const rows = [row('old', null, OLD), row('new', null, NEW), row('mid', null, MID)];
    expect(defaultBrowseContextInstance(rows)).toBe('new');
  });

  it('treats unreadable timestamps as missing, not as the newest', () => {
    const rows = [row('garbage', 'not-a-date', 'nope'), row('real', null, OLD)];
    expect(defaultBrowseContextInstance(rows)).toBe('real');
  });

  it('is deterministic when every timestamp is unreadable', () => {
    const rows = [row('first', '', ''), row('second', 'x', 'y')];
    expect(defaultBrowseContextInstance(rows)).toBe('first');
  });

  it('returns a single instance', () => {
    expect(defaultBrowseContextInstance([row('only', null, OLD)])).toBe('only');
  });
});

describe('pickBrowseContextInstance', () => {
  const rows = [row('old', OLD, OLD), row('recent', NEW, OLD)];

  it('is empty with no instances, whatever was offered or chosen', () => {
    expect(pickBrowseContextInstance({ offered: 'x', choice: 'x', rows: [] })).toBe('');
  });

  it('defaults to the most recently used instance when nothing was chosen', () => {
    expect(pickBrowseContextInstance({ offered: null, choice: null, rows })).toBe('recent');
  });

  it('an instance handed over from My Instances wins over the default', () => {
    expect(pickBrowseContextInstance({ offered: 'old', choice: null, rows })).toBe('old');
  });

  it('keeps a choice of "No instance context" instead of re-defaulting', () => {
    expect(pickBrowseContextInstance({ offered: null, choice: '', rows })).toBe('');
  });

  it('keeps an earlier explicit choice', () => {
    expect(pickBrowseContextInstance({ offered: null, choice: 'old', rows })).toBe('old');
  });

  it('ignores an offered id that no longer exists', () => {
    expect(pickBrowseContextInstance({ offered: 'gone', choice: null, rows })).toBe('recent');
  });

  it('ignores a remembered choice that no longer exists', () => {
    expect(pickBrowseContextInstance({ offered: null, choice: 'deleted', rows })).toBe('recent');
  });

  it('a missing offer still lets a valid choice through', () => {
    expect(pickBrowseContextInstance({ offered: 'gone', choice: 'old', rows })).toBe('old');
  });

  it('does not treat a blank id as an instance', () => {
    expect(pickBrowseContextInstance({ offered: '', choice: null, rows })).toBe('recent');
  });
});

describe('browse context choice memory', () => {
  beforeEach(() => saveBrowseContextChoice(''));

  it('remembers an explicit choice, including "none"', () => {
    saveBrowseContextChoice('fabric-121');
    expect(peekBrowseContextChoice()).toBe('fabric-121');
    saveBrowseContextChoice('');
    expect(peekBrowseContextChoice()).toBe('');
  });
});

describe('browseResultsStale', () => {
  const base = { resultsKey: 'k1', queryKey: 'k1', query: 'create', debouncedQuery: 'create' };

  it('is fresh when the results match the controls', () => {
    expect(browseResultsStale(base)).toBe(false);
  });

  it('is stale before any results were loaded', () => {
    expect(browseResultsStale({ ...base, resultsKey: null })).toBe(true);
  });

  it('is stale when the filters changed since the results were loaded', () => {
    expect(browseResultsStale({ ...base, queryKey: 'k2' })).toBe(true);
  });

  it('is stale while a typed query is still waiting out its debounce', () => {
    expect(browseResultsStale({ ...base, query: 'Create', debouncedQuery: 'Crea' })).toBe(true);
  });

  it('treats surrounding whitespace as the same query', () => {
    expect(browseResultsStale({ ...base, query: '  create ', debouncedQuery: 'create' })).toBe(false);
  });
});

describe('shouldOfferModrinthSearch', () => {
  const off = { enabled: false, unavailableReason: null };
  const empty = {
    provider: off,
    query: 'create',
    sort: 'net_score',
    resultCount: 0,
    loading: false,
    hasError: false,
  };

  it('offers Modrinth under an empty search while it is off', () => {
    expect(shouldOfferModrinthSearch(empty)).toBe(true);
  });

  it('does not offer when the provider has not been read', () => {
    expect(shouldOfferModrinthSearch({ ...empty, provider: null })).toBe(false);
  });

  it('does not offer when Modrinth is already on', () => {
    expect(shouldOfferModrinthSearch({ ...empty, provider: { enabled: true } })).toBe(false);
  });

  it('does not offer when privacy or lockdown blocks it', () => {
    expect(shouldOfferModrinthSearch({
      ...empty,
      provider: { enabled: false, unavailableReason: 'Privacy Lockdown Mode is on.' },
    })).toBe(false);
  });

  it('does not offer without a search (blank or whitespace query)', () => {
    expect(shouldOfferModrinthSearch({ ...empty, query: '' })).toBe(false);
    expect(shouldOfferModrinthSearch({ ...empty, query: '   ' })).toBe(false);
  });

  it('does not offer when there are results', () => {
    expect(shouldOfferModrinthSearch({ ...empty, resultCount: 1 })).toBe(false);
  });

  it('does not offer while loading or after a failed search', () => {
    expect(shouldOfferModrinthSearch({ ...empty, loading: true })).toBe(false);
    expect(shouldOfferModrinthSearch({ ...empty, hasError: true })).toBe(false);
  });

  it('does not offer under For You, which never queries providers', () => {
    expect(shouldOfferModrinthSearch({ ...empty, sort: 'for_you' })).toBe(false);
  });
});
