/**
 * The bag has to be a place you can look inside and take things out of.
 *
 * Three things are worth pinning down. The overlays are portalled to <body>:
 * they are `position: fixed`, and ambience puts a `backdrop-filter` on `main`,
 * which makes `main` the containing block for fixed descendants — rendered in
 * place, the quick look was pinned to the top of the scrolled page instead of
 * the middle of the screen. The bag lists what is in it, with a way out. And a
 * pick that is already in the bag offers to come back out, rather than showing
 * a dead "In your bag" label that leaves no way to change your mind.
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
      items={[item()]}
      instanceVersion="1.21.8"
      ownedIds={new Set()}
      onOpenMod={vi.fn()}
      onExit={vi.fn()}
      {...props}
    />,
  );
}

/** Open the quick look for the first tile. */
function openQuickLook(name = 'Sodium') {
  fireEvent.click(screen.getByRole('button', { name: `Quick look at ${name}` }));
}

describe('the Bazaar bag', () => {
  beforeEach(() => {
    window.localStorage.removeItem(STORAGE_KEY);
  });

  it('portals the quick look out of the scrolling page', () => {
    const { container } = renderBazaar();
    openQuickLook();
    const dialog = screen.getByRole('dialog', { name: 'Sodium' });
    // Not inside the component's own subtree — anything fixed in there inherits
    // `main`'s containing block once ambience is on.
    expect(container.querySelector('.bazaar-detail-scrim')).toBeNull();
    expect(dialog.closest('.bazaar')).toBeNull();
    expect(dialog.closest('body')).toBe(document.body);
  });

  it('shows what is in the bag and lets it back out', () => {
    renderBazaar();
    openQuickLook();
    fireEvent.click(screen.getByTestId('bazaar-detail-bag'));

    // The badge counts picks, and the bag lists them.
    expect(within(screen.getByTestId('bazaar-open-bag')).getByText('1')).toBeTruthy();
    fireEvent.click(screen.getByTestId('bazaar-open-bag'));
    const bag = screen.getByTestId('bazaar-bag');
    expect(within(bag).getByText('Sodium')).toBeTruthy();

    fireEvent.click(within(bag).getByRole('button', { name: 'Remove Sodium from my bag' }));
    expect(within(screen.getByTestId('bazaar-bag')).queryByText('Sodium')).toBeNull();
    expect(within(screen.getByTestId('bazaar-open-bag')).getByText('0')).toBeTruthy();
  });

  it('offers to take a picked item back out from the quick look', () => {
    renderBazaar();
    openQuickLook();
    fireEvent.click(screen.getByTestId('bazaar-detail-bag'));

    openQuickLook();
    const toggle = screen.getByTestId('bazaar-detail-bag');
    expect(toggle.textContent).toBe('Remove from my bag');
    expect((toggle as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(toggle);
    expect(within(screen.getByTestId('bazaar-open-bag')).getByText('0')).toBeTruthy();
  });

  it('will not offer to unbag something that is already installed', () => {
    renderBazaar({ ownedIds: new Set(['sodium']) });
    openQuickLook();
    const toggle = screen.getByTestId('bazaar-detail-bag') as HTMLButtonElement;
    expect(toggle.textContent).toBe('Already in this instance');
    expect(toggle.disabled).toBe(true);
    // Installed content is not a pick, so it does not inflate the bag badge.
    expect(within(screen.getByTestId('bazaar-open-bag')).getByText('0')).toBeTruthy();
  });

  it('keeps picks made on another stall, and says why they cannot install yet', () => {
    const onInstallBag = vi.fn();
    const { rerender } = renderBazaar({ onInstallBag });
    openQuickLook();
    fireEvent.click(screen.getByTestId('bazaar-detail-bag'));

    // Switching stalls refetches Browse with a different content type, so the
    // picked mod is no longer in `items`.
    rerender(
      <BrowseBazaar
        items={[item({ id: 'complementary', name: 'Complementary', contentType: 'shader' })]}
        instanceVersion="1.21.8"
        ownedIds={new Set()}
        onOpenMod={vi.fn()}
        onExit={vi.fn()}
        onInstallBag={onInstallBag}
      />,
    );
    fireEvent.click(screen.getByTestId('bazaar-open-bag'));
    const bag = screen.getByTestId('bazaar-bag');
    expect(within(bag).getByText('Sodium')).toBeTruthy();
    expect(within(bag).getByText('Open the Mods stall to install this one')).toBeTruthy();
    expect((screen.getByTestId('bazaar-install-bag') as HTMLButtonElement).disabled).toBe(true);
    expect(onInstallBag).not.toHaveBeenCalled();
  });

  it('hands the bag to the reviewed batch install flow', () => {
    const onInstallBag = vi.fn();
    renderBazaar({ onInstallBag });
    openQuickLook();
    fireEvent.click(screen.getByTestId('bazaar-detail-bag'));
    fireEvent.click(screen.getByTestId('bazaar-open-bag'));
    fireEvent.click(screen.getByTestId('bazaar-install-bag'));
    expect(onInstallBag).toHaveBeenCalledTimes(1);
    expect(onInstallBag.mock.calls[0][0].map((it: BazaarItem) => it.id)).toEqual(['sodium']);
  });
});
