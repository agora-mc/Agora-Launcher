import fs from 'fs';
import path from 'path';

// ────────────────────────────────────────────────────────────────────
// docs-web.json types
// ────────────────────────────────────────────────────────────────────

export interface DocPage {
  slug: string;
  title: string;
  description: string;
  path: string;
  content: string;
}

interface WebDocs {
  schema_version: number;
  generated_at: string;
  docs: DocPage[];
}

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
    _cached = { schema_version: 1, generated_at: '', docs: [] };
    return _cached;
  }
  const raw = fs.readFileSync(filePath, 'utf-8');
  _cached = JSON.parse(raw) as WebDocs;
  if (!_cached.schema_version || !Array.isArray(_cached.docs)) {
    throw new Error('docs-web.json has an unexpected schema.');
  }
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
