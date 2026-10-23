import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { BrowseStats } from './BrowseContextLabels';
import type { BrowseItem } from './types';

function curated(overrides: Partial<BrowseItem> = {}): BrowseItem {
  return {
    id: 'xaeros-minimap',
    source: 'curated',
    registryItem: null,
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
    upvotes: 7,
    downvotes: 2,
    netScore: 5,
    supportedVersions: [],
    sourcePageUrl: null,
    ...overrides,
  };
}

describe('BrowseStats score line on a card', () => {
  it('shows the numbers as plain, read-only text', () => {
    const { container } = render(<BrowseStats item={curated()} />);

    expect(screen.getByText('7 up')).toBeInTheDocument();
    expect(screen.getByText('2 down')).toBeInTheDocument();
    // The net score is kept out of the card face (jargon) and into the tooltip.
    expect(screen.queryByText(/Score/)).toBeNull();
    expect(container.querySelector('.browse-stats')).toHaveAttribute('title', expect.stringContaining('Net score 5'));
    // The vote arrows were the things that looked clickable.
    expect(container.querySelectorAll('svg')).toHaveLength(0);
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.queryByRole('link')).toBeNull();
  });

  it('falls back to zero when a vote count is missing', () => {
    render(<BrowseStats item={curated({ upvotes: null, downvotes: null, netScore: 0 })} />);
    expect(screen.getByText('0 up')).toBeInTheDocument();
    expect(screen.queryByText(/Score/)).toBeNull();
  });

  it('keeps the download and follower line for other sources', () => {
    render(<BrowseStats item={curated({ source: 'modrinth', netScore: null, downloads: 1200, follows: 30 })} />);
    expect(screen.getByText('1,200 downloads · 30 followers')).toBeInTheDocument();
  });
});
