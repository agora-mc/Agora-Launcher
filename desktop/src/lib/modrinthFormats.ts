/**
 * Modrinth tags each project version with `loaders`. For mods that is the mod
 * loader, but other content uses its own tags: resource packs `minecraft`,
 * data packs `datapack`, shaders `iris`/`optifine`/`canvas`/`vanilla`. A single
 * project can ship several formats (a mod with a companion data pack), so the
 * format has to be read from the versions, not from the project type.
 */
export type ModrinthFormat = 'mod' | 'datapack';

const NON_MOD_LOADERS = new Set(['minecraft', 'datapack', 'iris', 'optifine', 'canvas', 'vanilla']);

/** The install format a single version's loader tags describe. */
export function versionFormat(loaders: readonly string[]): ModrinthFormat {
  const tags = loaders.map((loader) => loader.toLowerCase());
  if (tags.some((tag) => !NON_MOD_LOADERS.has(tag))) return 'mod';
  return tags.includes('datapack') ? 'datapack' : 'mod';
}

/** The formats a project's versions come in, mods first. */
export function projectFormats(versions: ReadonlyArray<{ loaders: readonly string[] }>): ModrinthFormat[] {
  const found = new Set(versions.map((version) => versionFormat(version.loaders)));
  return (['mod', 'datapack'] as const).filter((format) => found.has(format));
}

/**
 * The content type to install a version as. A project typed resource pack or
 * shader stays that; a mod-typed project's data-pack versions install as data
 * packs.
 */
export function installContentType(projectContentType: string, loaders: readonly string[]): string {
  if (projectContentType !== 'mod') return projectContentType;
  return versionFormat(loaders) === 'datapack' ? 'datapack' : 'mod';
}

/** User-facing name of a content type. */
export function contentTypeLabel(contentType: string): string {
  switch (contentType) {
    case 'resourcepack': return 'resource pack';
    case 'shader': return 'shader';
    case 'datapack': return 'data pack';
    case 'world': return 'world';
    case 'pack': return 'modpack';
    default: return 'mod';
  }
}
