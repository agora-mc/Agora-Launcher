/**
 * Browse cards are the primary targets on the page a controller user reaches
 * first, and until now they were `<article>`/`<li>` elements with an `onClick`
 * and no way to focus them at all.
 *
 * The subtlety worth a test is the *other* direction: `onToggleSelect` is
 * optional, and a card without it is not interactive. Making those focusable
 * unconditionally would plant a dead stop in every navigation sequence on the
 * page — reachable, focusable, and does nothing when activated.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { BrowseItem } from './types';
import { BrowseListCard } from './BrowseListCard';
import { BrowseTileCard } from './BrowseTileCard';

function item(overrides: Partial<BrowseItem> = {}): BrowseItem {
  return {
    id: 'sodium',
    source: 'curated',
    registryItem: null,
    modrinthResult: null,
    name: 'Sodium',
    iconUrl: null,
    description: 'Rendering optimisation.',
    contentType: 'mod',
    heroImageUrl: null,
    author: 'jellysquid3',
    categories: ['performance'],
    downloads: 100,
    follows: 10,
    upvotes: null,
    downvotes: null,
    netScore: null,
    supportedVersions: ['1.21.8'],
    sourcePageUrl: null,
    ...overrides,
  };
}

const cards = [
  ['BrowseTileCard', BrowseTileCard],
  ['BrowseListCard', BrowseListCard],
] as const;

describe.each(cards)('%s focusability', (_name, Card) => {
  it('is reachable and activatable when selectable', () => {
    const onToggleSelect = vi.fn();
    render(<Card item={item()} context={null} onToggleSelect={onToggleSelect} />);

    const card = screen.getByRole('button', { name: /sodium/i });
    expect(card).toHaveAttribute('tabindex', '0');

    card.focus();
    expect(document.activeElement).toBe(card);

    fireEvent.keyDown(card, { key: 'Enter' });
    expect(onToggleSelect).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(card, { key: ' ' });
    expect(onToggleSelect).toHaveBeenCalledTimes(2);
  });

  it('reflects selection state to assistive technology', () => {
    render(<Card item={item()} context={null} selected onToggleSelect={vi.fn()} />);

    expect(screen.getByRole('button', { name: /sodium/i })).toHaveAttribute('aria-pressed', 'true');
  });

  it('is not focusable at all when it is not selectable', () => {
    const { container } = render(<Card item={item()} context={null} />);

    const card = container.firstElementChild;
    expect(card).not.toBeNull();
    expect(card).not.toHaveAttribute('tabindex');
    expect(card).not.toHaveAttribute('role', 'button');
  });

  it('does not double-fire when a nested control is activated', () => {
    const onToggleSelect = vi.fn();
    const { container } = render(
      <Card item={item()} context={null} onToggleSelect={onToggleSelect} />,
    );

    const nested = container.querySelector('button, a[href]');
    // Only meaningful when the card actually renders an inner control; the
    // guard being tested is `event.target !== event.currentTarget`.
    if (nested) {
      fireEvent.keyDown(nested, { key: 'Enter' });
      expect(onToggleSelect).not.toHaveBeenCalled();
    }
  });
});
