import { describe, expect, it } from 'vitest';
import { onBrowseSearchRequest, requestBrowseSearch, takeBrowseSearchRequest } from './browseSearchRequest';

describe('browse search request', () => {
  it('parks a request until Browse takes it, once', () => {
    requestBrowseSearch('connectormod');
    expect(takeBrowseSearchRequest()).toBe('connectormod');
    expect(takeBrowseSearchRequest()).toBeNull();
  });

  it('delivers to a mounted listener', () => {
    const seen: string[] = [];
    const off = onBrowseSearchRequest((q) => seen.push(q));
    requestBrowseSearch('sodium');
    off();
    expect(seen).toEqual(['sodium']);
    expect(takeBrowseSearchRequest()).toBeNull();
  });
});
