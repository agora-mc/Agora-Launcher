import { GITHUB_API_RELEASES_URL } from '@/lib/site';

export interface ReleaseAsset {
  name: string;
  browser_download_url: string;
  size: number;
}

export interface GitHubRelease {
  tag_name: string;
  assets?: ReleaseAsset[];
  draft?: boolean;
  prerelease?: boolean;
  published_at?: string | null;
}

/** Only the fields the download UI needs, safe to serialize into the page. */
export interface DesktopRelease {
  tag: string;
  assets: ReleaseAsset[];
}

export type DetectedOS = 'windows' | 'macos' | 'linux' | 'unknown';

/**
 * Compare two `v*` tags newest-first.
 *
 * Ordering by `published_at` alone is not enough: editing an old release
 * re-stamps it, which is how the button can end up pointing at an ancient
 * installer. Semver ordering is the intent; the timestamp is only a
 * tie-break for tags that compare equal (e.g. `v1.2.3` and `v1.2.3-1`).
 */
function compareReleases(a: GitHubRelease, b: GitHubRelease): number {
  const parse = (tag: string) =>
    tag
      .replace(/^v/, '')
      .split(/[.\-+]/)
      .map((part) => {
        const n = Number.parseInt(part, 10);
        return Number.isNaN(n) ? -1 : n;
      });

  const av = parse(a.tag_name);
  const bv = parse(b.tag_name);
  for (let i = 0; i < Math.max(av.length, bv.length); i += 1) {
    const diff = (bv[i] ?? 0) - (av[i] ?? 0);
    if (diff !== 0) return diff;
  }

  const at = Date.parse(a.published_at ?? '');
  const bt = Date.parse(b.published_at ?? '');
  return (Number.isNaN(bt) ? 0 : bt) - (Number.isNaN(at) ? 0 : at);
}

export function selectLatestDesktopRelease(releases: unknown): GitHubRelease | null {
  if (!Array.isArray(releases)) return null;

  const desktopReleases = releases.filter(
    (release): release is GitHubRelease =>
      typeof release === 'object' &&
      release !== null &&
      typeof (release as GitHubRelease).tag_name === 'string' &&
      (release as GitHubRelease).tag_name.startsWith('v') &&
      !(release as GitHubRelease).draft &&
      !(release as GitHubRelease).prerelease &&
      Array.isArray((release as GitHubRelease).assets)
  );

  return desktopReleases.sort(compareReleases)[0] ?? null;
}

export function toDesktopRelease(release: GitHubRelease | null): DesktopRelease | null {
  if (!release) return null;
  return {
    tag: release.tag_name,
    assets: (release.assets ?? []).map((asset) => ({
      name: asset.name,
      browser_download_url: asset.browser_download_url,
      size: asset.size,
    })),
  };
}

export function pickAsset(os: DetectedOS, assets: ReleaseAsset[]): ReleaseAsset | null {
  if (assets.length === 0) return null;
  const findMatch = (pred: (name: string) => boolean) =>
    assets.find((a) => pred(a.name.toLowerCase()));

  if (os === 'windows') {
    return findMatch((n) => n.endsWith('.msi')) || findMatch((n) => n.endsWith('.exe')) || null;
  }
  if (os === 'macos') {
    return findMatch((n) => n.endsWith('.dmg')) || null;
  }
  if (os === 'linux') {
    return findMatch((n) => n.endsWith('.appimage')) || findMatch((n) => n.endsWith('.deb')) || null;
  }
  return null;
}

/**
 * Resolve the newest desktop release at build time.
 *
 * The download button also re-checks from the browser, so a site built before
 * a release still picks the new one up. This exists so the button is correct
 * even when that request cannot happen: GitHub's unauthenticated API allows
 * 60 requests per hour per IP, which shared NAT and corporate proxies burn
 * through, and the site is otherwise fully static with no origin to ask.
 *
 * Failure is not fatal — an offline `next build` falls back to the runtime
 * fetch, exactly as before this was baked in.
 */
export async function fetchLatestDesktopRelease(): Promise<DesktopRelease | null> {
  const token = process.env.GITHUB_TOKEN;
  try {
    const res = await fetch(GITHUB_API_RELEASES_URL, {
      headers: {
        Accept: 'application/vnd.github+json',
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
    });
    if (!res.ok) {
      console.warn(`[releases] GitHub API returned ${res.status}; download button will resolve in the browser.`);
      return null;
    }
    return toDesktopRelease(selectLatestDesktopRelease(await res.json()));
  } catch (err) {
    console.warn(`[releases] Could not reach the GitHub API (${err}); download button will resolve in the browser.`);
    return null;
  }
}
