import { useEffect, useState } from 'react';
import { fetchCommunityImage } from '../lib/tauri';

/** Hosts the webview's CSP `img-src` admits directly (see tauri.conf.json). */
const DIRECT_HOSTS = ['cdn.modrinth.com', 'modrinthcdn.com', 'githubusercontent.com'];

/** One fetch per URL for the session, shared by every page that shows it. */
const fetched = new Map<string, Promise<string>>();

/** How an About-text image source can be displayed: as is, through core, or not at all. */
export function imageRoute(src: string | undefined): 'direct' | 'fetch' | 'none' {
  if (!src) return 'none';
  if (src.startsWith('data:image/') || src.startsWith('blob:')) return 'direct';
  let url: URL;
  try {
    url = new URL(src);
  } catch {
    return 'none';
  }
  if (url.protocol !== 'https:') return 'none';
  const host = url.hostname;
  return DIRECT_HOSTS.some((allowed) => host === allowed || host.endsWith(`.${allowed}`)) ? 'direct' : 'fetch';
}

/**
 * An image inside community-written About text. Images on hosts the CSP does
 * not admit are fetched by core, which only returns them once their bytes are
 * confirmed to be an image. A failed or rejected image is hidden instead of
 * leaving a broken-image placeholder.
 */
export function HideOnErrorImage({ node: _node, src, ...props }: React.ImgHTMLAttributes<HTMLImageElement> & { node?: unknown }) {
  const route = imageRoute(typeof src === 'string' ? src : undefined);
  const [resolved, setResolved] = useState<string | null>(route === 'direct' ? (src as string) : null);
  const [failed, setFailed] = useState(route === 'none');

  useEffect(() => {
    if (route !== 'fetch' || typeof src !== 'string') return;
    let cancelled = false;
    let pending = fetched.get(src);
    if (!pending) {
      pending = fetchCommunityImage(src);
      fetched.set(src, pending);
      pending.catch(() => fetched.delete(src));
    }
    pending.then(
      (dataUrl) => { if (!cancelled) setResolved(dataUrl); },
      () => { if (!cancelled) setFailed(true); },
    );
    return () => { cancelled = true; };
  }, [route, src]);

  if (failed || !resolved) return null;
  return (
    // eslint-disable-next-line jsx-a11y/alt-text
    <img
      {...props}
      src={resolved}
      loading="lazy"
      className="max-w-full h-auto rounded-lg"
      onError={() => setFailed(true)}
    />
  );
}
