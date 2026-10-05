import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';

const fetchCommunityImage = vi.fn();
vi.mock('../lib/tauri', () => ({ fetchCommunityImage: (url: string) => fetchCommunityImage(url) }));

import { GalleryGrid } from './GalleryGrid';

const URLS = [
  'https://cdn.modrinth.com/data/a/images/one.png',
  'https://cdn.modrinth.com/data/a/images/two.png',
];

describe('GalleryGrid', () => {
  it('opens a larger view when a screenshot is clicked and closes it with Escape', async () => {
    render(<GalleryGrid urls={URLS} name="Sodium" />);
    expect(screen.queryByRole('dialog')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'View Sodium screenshot 2 larger' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Sodium screenshot 2')).toBeTruthy();
    expect(within(dialog).getByAltText('Sodium screenshot 2').getAttribute('src')).toBe(URLS[1]);

    fireEvent.click(within(dialog).getByRole('button', { name: 'Next' }));
    expect(within(dialog).getByAltText('Sodium screenshot 1').getAttribute('src')).toBe(URLS[0]);

    fireEvent.keyDown(dialog, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('shows images on other hosts only through the verified fetch', async () => {
    fetchCommunityImage.mockResolvedValue('data:image/png;base64,AAAA');
    render(<GalleryGrid urls={['https://example.com/shot.png']} name="Pack" />);
    fireEvent.click(await screen.findByRole('button', { name: 'View Pack screenshot 1 larger' }));
    const dialog = await screen.findByRole('dialog');
    await waitFor(() => expect(within(dialog).getByAltText('Pack screenshot 1').getAttribute('src')).toBe('data:image/png;base64,AAAA'));
    expect(fetchCommunityImage).toHaveBeenCalledWith('https://example.com/shot.png');
  });
});
