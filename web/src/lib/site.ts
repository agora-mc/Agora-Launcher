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
