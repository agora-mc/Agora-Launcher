// ────────────────────────────────────────────────────────────────────
// Content type helpers
//
// Client-safe: this module is imported by both server components and the
// client-side catalog navigation, so it must not pull in `server-only`
// (see `db.ts`, which re-exports everything here).
// ────────────────────────────────────────────────────────────────────

export const CONTENT_TYPES = [
  'mod',
  'pack',
  'shader',
  'resourcepack',
  'server',
  'datapack',
  'world',
] as const;

export type ContentType = (typeof CONTENT_TYPES)[number];

export function isContentType(value: string): value is ContentType {
  return CONTENT_TYPES.includes(value as ContentType);
}

export function contentTypeLabel(type: ContentType): string {
  switch (type) {
    case 'mod':
      return 'Mods';
    case 'pack':
      return 'Modpacks';
    case 'shader':
      return 'Shaders';
    case 'resourcepack':
      return 'Resource Packs';
    case 'server':
      return 'Servers';
    case 'datapack':
      return 'Datapacks';
    case 'world':
      return 'Worlds';
    default:
      return type;
  }
}

export function contentTypePath(type: ContentType): string {
  return `/${type}s`;
}

export function contentTypeFromPath(pathSegment: string): ContentType | null {
  for (const t of CONTENT_TYPES) {
    if (pathSegment === `${t}s`) return t;
  }
  return null;
}

/** Catalog section paths, in the order they appear in the navigation. */
export const CATALOG_PATHS: string[] = CONTENT_TYPES.map(contentTypePath);

/** Where the primary "Catalog" link lands. */
export const CATALOG_HOME = contentTypePath('mod');

/**
 * Normalize a pathname for route matching.
 *
 * The site is a static export, so the same page is reachable as `/mods` and as
 * `/mods.html` (and with a trailing slash) depending on how it is served.
 * Matching would silently miss on those variants without this.
 */
export function normalizePathname(pathname: string): string {
  const withoutHtml = pathname.replace(/\.html$/, '');
  const trimmed = withoutHtml.replace(/\/+$/, '');
  return trimmed === '' ? '/' : trimmed;
}

/** Whether a pathname belongs to a catalog section (listing or item detail). */
export function isCatalogPath(pathname: string): boolean {
  const path = normalizePathname(pathname);
  return CATALOG_PATHS.some((p) => path === p || path.startsWith(`${p}/`));
}
