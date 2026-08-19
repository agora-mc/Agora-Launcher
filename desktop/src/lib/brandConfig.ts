const repositorySlug = import.meta.env.VITE_AGORA_REPOSITORY?.trim();

// Fall back to the canonical repository so informational pages always render
// a working GitHub link even when the build env var is unset (e.g. e2e/dev).
const DEFAULT_REPOSITORY_SLUG = 'agora-mc/Agora-Launcher';

export const agoraRepositoryUrl = repositorySlug && /^[^/\s]+\/[^/\s]+$/.test(repositorySlug)
  ? `https://github.com/${repositorySlug}`
  : `https://github.com/${DEFAULT_REPOSITORY_SLUG}`;

export const agoraDiscordUrl = 'https://discord.gg/56tpsa2sTZ';

export const agoraSponsorsUrl = 'https://github.com/sponsors/jarjarpfeil';
