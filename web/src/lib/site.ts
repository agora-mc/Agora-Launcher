const repositorySlug = process.env.NEXT_PUBLIC_GITHUB_REPOSITORY || process.env.GITHUB_REPOSITORY;

if (!repositorySlug) {
  throw new Error('NEXT_PUBLIC_GITHUB_REPOSITORY or GITHUB_REPOSITORY must be configured');
}

export const GITHUB_REPO_URL = `https://github.com/${repositorySlug}`;

export const GITHUB_RELEASES_URL = `${GITHUB_REPO_URL}/releases`;

export const DISCORD_URL = 'https://discord.gg/56tpsa2sTZ';

export const SPONSORS_URL = 'https://github.com/sponsors/jarjarpfeil';

// Registry snapshots are published as non-prerelease `registry-*` releases, so
// GitHub's `/releases/latest` endpoint points to a database asset rather than
// the desktop installer. Consumers should fetch this list and select a `v*`
// desktop release instead.
export const GITHUB_API_RELEASES_URL =
  `https://api.github.com/repos/${repositorySlug}/releases?per_page=100`;

/**
 * Homepage trailer.
 *
 * The video is a GitHub Release asset rather than a file in `web/public`, for
 * the same reason the registry ships that way: a ~15 MB binary committed here
 * would sit in git history forever, and every re-render would add another
 * copy that no `git clone` could ever avoid. Release assets are also outside
 * the Pages bandwidth budget.
 *
 * It hangs off the dedicated `site-assets` tag rather than a `v*` release, so
 * re-cutting an app release cannot break the homepage. That release is marked
 * pre-release purely so it never takes the "Latest" badge from a real one.
 *
 * The poster stays local — it is small, and it is what renders before anyone
 * presses play, so it should not depend on a second origin.
 *
 * To re-publish after a re-render:
 *   gh release upload site-assets agora-trailer.mp4 --clobber
 *
 * Set TRAILER_SRC to null to fall back to the framed placeholder; the hero
 * occupies the same box either way, so nothing shifts.
 */
export const TRAILER_SRC: string | null =
  'https://github.com/agora-mc/Agora-Launcher/releases/download/site-assets/agora-trailer.mp4';
export const TRAILER_POSTER: string | null = '/trailer-poster.jpg';
