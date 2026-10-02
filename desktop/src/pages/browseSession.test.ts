import { beforeEach, describe, expect, it } from 'vitest';
import {
  clearBrowseSnapshot,
  parkBrowseSnapshot,
  peekParkedBrowseFilter,
  pickDefaultPackRelease,
  saveBrowseSnapshot,
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
