/**
 * Whether the mod-loader filter means anything for what Browse is showing.
 *
 * Only mods and modpacks are tagged with a mod loader. Resource packs, shaders
 * and data packs carry their own tags (`minecraft`, `iris`, `datapack`), so
 * filtering them by the instance's loader hides nearly everything. "All types"
 * keeps the filter, as it always has.
 */
export function loaderFilterApplies(contentType: string | null): boolean {
  return contentType === null || contentType === 'mod' || contentType === 'pack';
}

/**
 * What the empty Browse state says when the instance's version or loader
 * filter is what hid the matches. Null when the filter hid nothing.
 */
export function hiddenResultsSentence(
  hidden: { count: number; more: boolean },
  mcVersion: string | null,
  loader: string | null,
): string | null {
  if (hidden.count <= 0) return null;
  const scope = [mcVersion, loader ? loader.charAt(0).toUpperCase() + loader.slice(1) : null]
    .filter((part): part is string => Boolean(part))
    .join(' ');
  const noun = hidden.count === 1 ? 'result' : 'results';
  const prefix = hidden.more ? 'At least ' : '';
  return `No results for ${scope || 'this instance'}. ${prefix}${hidden.count} ${noun} for other versions or loaders.`;
}

/**
 * Why a content type has no results regardless of filters, if that is known.
 * Modrinth has no world project type, and no other source Agora browses lists
 * worlds, so an empty world catalog is expected rather than a failed search.
 */
export function contentTypeUnavailableReason(contentType: string | null): string | null {
  if (contentType === 'world') {
    return 'World downloads are not available from the sources Agora browses. Modrinth does not host worlds, so this list will stay empty. To play a world, copy its folder into the instance’s saves folder, or import the instance it lives in from another launcher.';
  }
  return null;
}
