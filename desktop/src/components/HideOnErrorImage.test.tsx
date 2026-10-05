import { describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';

const fetchCommunityImage = vi.fn();
vi.mock('../lib/tauri', () => ({ fetchCommunityImage: (url: string) => fetchCommunityImage(url) }));

import { HideOnErrorImage, imageRoute } from './HideOnErrorImage';

describe('imageRoute', () => {
  it('loads CSP-allowed hosts directly and fetches other HTTPS hosts through core', () => {
    expect(imageRoute('https://cdn.modrinth.com/data/a/icon.png')).toBe('direct');
    expect(imageRoute('https://raw.githubusercontent.com/a/b/c.png')).toBe('direct');
    expect(imageRoute('https://img.shields.io/badge/x-y-green')).toBe('fetch');
    expect(imageRoute('https://evil-githubusercontent.com/x.png')).toBe('fetch');
  });

  it('drops sources that cannot be shown', () => {
    expect(imageRoute('http://example.com/a.png')).toBe('none');
    expect(imageRoute('relative/a.png')).toBe('none');
    expect(imageRoute(undefined)).toBe('none');
  });
});

describe('HideOnErrorImage', () => {
  it('shows the verified data URL core returns', async () => {
    fetchCommunityImage.mockResolvedValueOnce('data:image/svg+xml;base64,PHN2Zz4=');
    render(<HideOnErrorImage src="https://img.shields.io/badge/ok" alt="badge" />);
    await waitFor(() => expect(screen.getByAltText('badge')).toHaveAttribute('src', 'data:image/svg+xml;base64,PHN2Zz4='));
  });

  it('renders nothing when core rejects the file', async () => {
    fetchCommunityImage.mockRejectedValueOnce({ code: 'ERR_NOT_AN_IMAGE' });
    const { container } = render(<HideOnErrorImage src="https://example.com/not-image" alt="x" />);
    await waitFor(() => expect(fetchCommunityImage).toHaveBeenCalledWith('https://example.com/not-image'));
    expect(container.querySelector('img')).toBeNull();
  });
});
