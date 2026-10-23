import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { BrowseListCard } from './BrowseListCard';
import type { BrowseItem } from './types';

function providerItem(downloadSourcesJson: string | null): BrowseItem {
  return {
    id: 'xaeros-minimap',
    source: 'modrinth',
    providerId: 'modrinth',
    providerTitle: 'Modrinth',
    registryItem: {
      id: 'xaeros-minimap',
      download_strategy: 'github_release',
      source_identifier: 'owner/repo',
      modrinth_id: null,
      download_sources_json: downloadSourcesJson,
    } as unknown as BrowseItem['registryItem'],
    modrinthResult: null,
    name: "Xaero's Minimap",
    iconUrl: null,
    description: null,
    contentType: 'mod',
    heroImageUrl: null,
    author: null,
    categories: [],
    downloads: null,
    follows: null,
    upvotes: null,
    downvotes: null,
    netScore: null,
    supportedVersions: [],
    sourcePageUrl: null,
  };
}

describe('BrowseListCard source line', () => {
  it('says plainly how many backup download sources there are', () => {
    const json = JSON.stringify([
      { strategy: 'github_release', identifier: 'owner/repo' },
      { strategy: 'modrinth_id', identifier: 'abc' },
    ]);
    const { container } = render(<BrowseListCard item={providerItem(json)} context={null} />);

    const line = container.querySelector('.browse-card-source');
    expect(line).toHaveTextContent('Modrinth · downloads from GitHub Release, 1 backup source');
    // The old jargon must not come back.
    expect(screen.queryByText(/fallback/)).toBeNull();
    expect(screen.queryByText(/\+1/)).toBeNull();
  });

  it('pluralises the backup count', () => {
    const json = JSON.stringify([
      { strategy: 'github_release', identifier: 'owner/repo' },
      { strategy: 'modrinth_id', identifier: 'abc' },
      { strategy: 'direct_hash', identifier: 'https://example.invalid/a.jar' },
    ]);
    const { container } = render(<BrowseListCard item={providerItem(json)} context={null} />);
    expect(container.querySelector('.browse-card-source')).toHaveTextContent('2 backup sources');
  });

  it('names only the source when there is no backup', () => {
    const json = JSON.stringify([{ strategy: 'github_release', identifier: 'owner/repo' }]);
    const { container } = render(<BrowseListCard item={providerItem(json)} context={null} />);
    expect(container.querySelector('.browse-card-source')).toHaveTextContent(/^Modrinth · GitHub Release$/);
  });
});
