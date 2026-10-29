/**
 * Loading and copy for the Bazaar shelf.
 *
 * Browse refetches when the stall changes, and until that first page lands the
 * shelf has nothing to show. The empty-stall sentence must not appear during
 * that wait, because it tells the user the stall really is empty.
 */
import { fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { BrowseBazaar } from './BrowseBazaar';
import { STORAGE_KEY, type BazaarItem } from './bazaar-model';

vi.mock('../../lib/tauri', () => ({ getSetting: vi.fn(async () => false) }));

function item(over: Partial<BazaarItem> = {}): BazaarItem {
  return {
    id: 'sodium',
    name: 'Sodium',
    iconUrl: null,
    description: 'Rendering optimisation.',
    contentType: 'mod',
    author: 'jellysquid3',
    categories: ['optimization'],
    supportedVersions: ['1.21.8'],
    ...over,
  };
}

function renderBazaar(props: Partial<React.ComponentProps<typeof BrowseBazaar>> = {}) {
  return render(
    <BrowseBazaar
      items={[]}
      instanceVersion="1.21.8"
      ownedIds={new Set()}
      onOpenMod={vi.fn()}
      onExit={vi.fn()}
      {...props}
    />,
  );
}

const EMPTY_LINE = 'Nothing on this stall yet. Try another one.';

describe('the Bazaar while a stall is loading', () => {
  beforeEach(() => {
    window.localStorage.removeItem(STORAGE_KEY);
  });

  it('shows a loading line, not the empty-stall sentence, before the first load ends', () => {
    renderBazaar({ loading: true, items: [] });
    expect(screen.getByText('Loading this stall…')).toBeTruthy();
    expect(screen.queryByText(EMPTY_LINE)).toBeNull();
  });

  it('keeps the empty-stall sentence once the load has finished with nothing on the stall', () => {
    renderBazaar({ loading: false, items: [] });
    expect(screen.queryByText('Loading this stall…')).toBeNull();
    expect(screen.queryByText(EMPTY_LINE)).toBeNull();
    expect(screen.getByText(/Nothing on this stall right now/)).toBeTruthy();
  });

  it('treats an omitted loading prop as finished', () => {
    renderBazaar({ items: [] });
    expect(screen.queryByText('Loading this stall…')).toBeNull();
    expect(screen.getByText(/Nothing on this stall right now/)).toBeTruthy();
  });

  it('still shows items that are already on the shelf while a newer page loads', () => {
    renderBazaar({ loading: true, items: [item()] });
    expect(screen.getByRole('button', { name: 'Quick look at Sodium' })).toBeTruthy();
    expect(screen.queryByText('Loading this stall…')).toBeNull();
  });

  it('does not crank the Surprise me machine while the stall is loading', () => {
    renderBazaar({ loading: true, items: [] });
    const machine = screen.getByRole('button', { name: 'Surprise me' });
    expect(within(machine).getByText('Loading…')).toBeTruthy();
    fireEvent.click(machine);
    expect(within(machine).getByText('Loading…')).toBeTruthy();
    expect(within(machine).queryByText(/You got/)).toBeNull();
  });

  it('does not claim "you have got everything" on an empty stall', () => {
    renderBazaar({ items: [] });
    const machine = screen.getByRole('button', { name: 'Surprise me' });
    expect(within(machine).getByText('Nothing on this stall to crank yet')).toBeTruthy();
    fireEvent.click(machine);
    expect(within(machine).queryByText("You've got everything!")).toBeNull();
    expect(within(machine).getByText('Nothing on this stall to crank yet')).toBeTruthy();
  });
});

describe('the Bazaar copy', () => {
  beforeEach(() => {
    window.localStorage.removeItem(STORAGE_KEY);
  });

  it('explains what a stall is, next to the stall tabs', () => {
    renderBazaar({ items: [item()] });
    expect(screen.getByText('Each stall shows one kind of content. Pick one to browse it.')).toBeTruthy();
  });

  it('labels the header count as your bag, not a bare "picked"', () => {
    renderBazaar({ items: [item()] });
    const badge = screen.getByTestId('bazaar-open-bag');
    expect(badge.textContent).toContain('in your bag');
    expect(badge.textContent).not.toContain('picked');
  });
});
