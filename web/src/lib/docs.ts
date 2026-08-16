import fs from 'fs';
import path from 'path';

// ────────────────────────────────────────────────────────────────────
// docs-web.json types
// ────────────────────────────────────────────────────────────────────

/**
 * Top-level split the documentation navigation is organized by.
 * `internal` pages are published so cross-references never 404, but they are
 * kept out of the primary navigation.
 */
export type DocAudience = 'user' | 'developer' | 'internal';

export interface DocPage {
  slug: string;
  title: string;
  description: string;
  audience: DocAudience;
  /** Sub-heading within an audience, e.g. "Fix a problem". */
  group: string;
  path: string;
  /** Raw markdown exactly as committed. */
  content: string;
  /** Markdown with the leading H1 removed; the page header renders the title. */
  body: string;
}

export interface DocGroup {
  group: string;
  docs: DocPage[];
}

interface WebDocs {
  schema_version: number;
  generated_at: string;
  audience_order: DocAudience[];
  group_order: string[];
  docs: DocPage[];
}

export const AUDIENCE_LABELS: Record<DocAudience, string> = {
  user: 'For everyone using Agora',
  developer: 'For contributors and maintainers',
  internal: 'Working notes and archive',
};

const EMPTY_DOCS: WebDocs = {
  schema_version: 2,
  generated_at: '',
  audience_order: ['user', 'developer', 'internal'],
  group_order: [],
  docs: [],
};

// ────────────────────────────────────────────────────────────────────
// Loader
// ────────────────────────────────────────────────────────────────────

let _cached: WebDocs | null = null;

function getDocsWebJsonPath(): string {
  const fromEnv = process.env.DOCS_WEB_JSON_PATH;
  if (fromEnv) return path.resolve(fromEnv);
  return path.join(process.cwd(), '..', 'docs-web.json');
}

function loadDocs(): WebDocs {
  if (_cached) return _cached;
  const filePath = getDocsWebJsonPath();
  if (!fs.existsSync(filePath)) {
    console.warn(
      `docs-web.json not found at ${filePath}. Building with an empty docs index. ` +
      `Run "python scripts/build_docs_web.py" to populate it.`
    );
    _cached = EMPTY_DOCS;
    return _cached;
  }
  const raw = fs.readFileSync(filePath, 'utf-8');
  const parsed = JSON.parse(raw) as WebDocs;
  if (!parsed.schema_version || !Array.isArray(parsed.docs)) {
    throw new Error('docs-web.json has an unexpected schema.');
  }
  if (parsed.schema_version < 2) {
    throw new Error(
      `docs-web.json is schema_version ${parsed.schema_version}; the site needs 2 or newer. ` +
      'Re-run "python scripts/build_docs_web.py".'
    );
  }
  _cached = {
    ...parsed,
    audience_order: parsed.audience_order ?? EMPTY_DOCS.audience_order,
    group_order: parsed.group_order ?? [],
  };
  return _cached;
}

// ────────────────────────────────────────────────────────────────────
// Queries
// ────────────────────────────────────────────────────────────────────

export async function getAllDocs(): Promise<DocPage[]> {
  return loadDocs().docs;
}

export async function getDocBySlug(slug: string): Promise<DocPage | null> {
  return loadDocs().docs.find((doc) => doc.slug === slug) ?? null;
}

export async function getDocSlugs(): Promise<string[]> {
  return loadDocs().docs.map((doc) => doc.slug);
}

/** Documents for one audience, bucketed into their groups in display order. */
export async function getDocGroups(audience: DocAudience): Promise<DocGroup[]> {
  const { docs, group_order } = loadDocs();
  const buckets = new Map<string, DocPage[]>();
  for (const doc of docs) {
    if (doc.audience !== audience) continue;
    const bucket = buckets.get(doc.group);
    if (bucket) bucket.push(doc);
    else buckets.set(doc.group, [doc]);
  }
  // Groups the generator did not rank sort last, alphabetically, so a newly
  // added group is visible rather than silently dropped.
  const rank = (group: string) => {
    const index = group_order.indexOf(group);
    return index === -1 ? group_order.length : index;
  };
  return [...buckets.entries()]
    .map(([group, groupDocs]) => ({ group, docs: groupDocs }))
    .sort((a, b) => rank(a.group) - rank(b.group) || a.group.localeCompare(b.group));
}

/** Every audience in display order, each with its grouped documents. */
export async function getDocNav(): Promise<
  { audience: DocAudience; label: string; groups: DocGroup[] }[]
> {
  const { audience_order } = loadDocs();
  const sections = await Promise.all(
    audience_order.map(async (audience) => ({
      audience,
      label: AUDIENCE_LABELS[audience] ?? audience,
      groups: await getDocGroups(audience),
    }))
  );
  return sections.filter((section) => section.groups.length > 0);
}

// ────────────────────────────────────────────────────────────────────
// Navigation payload
// ────────────────────────────────────────────────────────────────────

export interface DocNavLink {
  slug: string;
  title: string;
}

export interface DocNavGroup {
  group: string;
  links: DocNavLink[];
}

export interface DocNavSection {
  audience: DocAudience;
  label: string;
  groups: DocNavGroup[];
}

/**
 * The same structure as {@link getDocNav}, reduced to slugs and titles.
 *
 * The sidebar is a client component, so anything handed to it is serialized
 * into the page payload. Every document's full markdown would be shipped to
 * the browser on every docs page if `DocPage` objects were passed directly.
 */
export async function getDocNavSections(): Promise<DocNavSection[]> {
  const sections = await getDocNav();
  return sections.map((section) => ({
    audience: section.audience,
    label: section.label,
    groups: section.groups.map((group) => ({
      group: group.group,
      links: group.docs.map((doc) => ({ slug: doc.slug, title: doc.title })),
    })),
  }));
}
