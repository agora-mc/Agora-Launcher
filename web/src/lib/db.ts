import 'server-only';
import fs from 'fs';
import path from 'path';

// ────────────────────────────────────────────────────────────────────
// Public registry-web.json types
// ────────────────────────────────────────────────────────────────────

export interface RegistryItem {
  id: string;
  name: string;
  author: string | null;
  content_type: string;
  curator_note: string;
  description: string | null;
  body_markdown: string | null;
  status: string;
  upvotes: number;
  downvotes: number;
  net_score: number;
  velocity: number;
  is_immune: boolean;
  categories: string[];
  community_categories: string[];
  icon_url: string | null;
  gallery_urls: string[];
  /// Preferred source's strategy — mirrors `download_sources[0].strategy`.
  download_strategy: string;
  source_identifier: string;
  /// Ordered download sources, best first. Absent on registry-web.json v1.
  download_sources?: DownloadSource[];
  sha256: string;
  page_url: string | null;
  modrinth_id: string | null;
  modrinth_url: string | null;
  github_repository_url: string | null;
  github_releases_url: string | null;
  github_issues_url: string | null;
  license: string | null;
  compatible_versions: { mc_version: string; loader: string; mod_version: string }[];
  date_added: string | null;
  source_updated_at: string | null;
  top_reviews: { author: string; rating: number; body: string; created_at: string }[];
}

/// One place an item's file can be fetched from. The launcher walks these in
/// order and installs from the first that is enabled and reachable.
export interface DownloadSource {
  strategy: string;
  identifier: string;
}

/// Ordered download sources for an item, best first.
///
/// v1 documents carry only the preferred source, so it is reconstructed from
/// the legacy fields — including the implicit Modrinth fallback a `modrinth_id`
/// gives a GitHub-hosted entry.
export function downloadSourcesOf(item: RegistryItem): DownloadSource[] {
  if (item.download_sources && item.download_sources.length > 0) {
    return item.download_sources;
  }
  const sources: DownloadSource[] = [
    { strategy: item.download_strategy, identifier: item.source_identifier },
  ];
  if (item.modrinth_id && item.download_strategy !== 'modrinth_id') {
    sources.push({ strategy: 'modrinth_id', identifier: item.modrinth_id });
  }
  return sources;
}

interface WebRegistry {
  schema_version: number;
  generated_at: string;
  items: RegistryItem[];
}

// ────────────────────────────────────────────────────────────────────
// Loader
// ────────────────────────────────────────────────────────────────────

let _cached: WebRegistry | null = null;

function getWebJsonPath(): string {
  const fromEnv = process.env.REGISTRY_WEB_JSON_PATH;
  if (fromEnv) return path.resolve(fromEnv);
  return path.join(process.cwd(), '..', 'registry-web.json');
}

function loadRegistry(): WebRegistry {
  if (_cached) return _cached;
  const filePath = getWebJsonPath();
  if (!fs.existsSync(filePath)) {
    console.warn(
      `registry-web.json not found at ${filePath}. Building with an empty registry. ` +
      `Run "python compiler/compile.py --skip-sign" to populate it.`
    );
    _cached = { schema_version: 1, generated_at: '', items: [] };
    return _cached;
  }
  const raw = fs.readFileSync(filePath, 'utf-8');
  _cached = JSON.parse(raw) as WebRegistry;
  if (!_cached.schema_version || !Array.isArray(_cached.items)) {
    throw new Error('registry-web.json has an unexpected schema.');
  }
  return _cached;
}

// ────────────────────────────────────────────────────────────────────
// Queries
// ────────────────────────────────────────────────────────────────────

export async function getAllItems(contentType?: string): Promise<RegistryItem[]> {
  const registry = loadRegistry();
  if (contentType) {
    return registry.items.filter((item) => item.content_type === contentType);
  }
  return registry.items;
}

export async function getItemById(id: string): Promise<RegistryItem | null> {
  const registry = loadRegistry();
  return registry.items.find((item) => item.id === id) ?? null;
}

export async function getItemIds(contentType?: string): Promise<string[]> {
  const items = await getAllItems(contentType);
  return items.map((item) => item.id);
}

export async function getReviews(
  itemId: string,
): Promise<{ author: string; rating: number; body: string; created_at: string }[]> {
  const item = await getItemById(itemId);
  if (!item) return [];
  return item.top_reviews ?? [];
}

// ────────────────────────────────────────────────────────────────────
// Content type helpers
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
