import {
  installTechnicSolderPack,
  installTechnicZipPack,
  technicPackDetail,
  type ImportResult,
} from '../../lib/tauri';

/**
 * Resolve a Technic pack by slug and install it via the tier-appropriate path.
 *
 * Extracted from the old standalone Technic panel so the unified Browse list can
 * install a Technic card without duplicating the tier logic. Core re-checks
 * consent on both entry points, so the `allowUnverifiedPacks` guard here is a
 * fast, friendly failure rather than the actual gate.
 */
export async function installTechnicPack(
  slug: string,
  allowUnverifiedPacks: boolean,
  /** Resolves the new instance's name once the pack's title is known; `null` cancels. */
  chooseName?: (title: string) => Promise<string | undefined | null>,
): Promise<ImportResult | null> {
  const detail = await technicPackDetail(slug);
  const instanceName = chooseName ? await chooseName(detail.title) : undefined;
  if (instanceName === null) return null;

  if (detail.tier === 'solder') {
    const solder = detail.solder;
    const build = detail.recommended_build;
    if (!solder || !build) {
      throw new Error('This Solder pack does not report a usable build to install.');
    }
    return installTechnicSolderPack(detail.slug, solder, build, instanceName);
  }

  if (!allowUnverifiedPacks) {
    throw new Error('Enable "Allow low security downloads" in Settings to install zip packs.');
  }
  const downloadUrl = detail.download_url;
  if (!downloadUrl) {
    throw new Error('This zip pack does not report a download URL.');
  }
  return installTechnicZipPack(
    detail.title,
    downloadUrl,
    null,
    detail.minecraft ?? '',
    '',
    '',
    instanceName,
  );
}
