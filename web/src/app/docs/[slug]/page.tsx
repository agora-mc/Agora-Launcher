import type { Metadata } from 'next';
import Link from 'next/link';
import { notFound } from 'next/navigation';
import MarkdownRenderer from '@/components/MarkdownRenderer';
import { GITHUB_REPO_URL } from '@/lib/site';
import { AUDIENCE_LABELS, getAllDocs, getDocBySlug } from '@/lib/docs';

export async function generateStaticParams() {
  const docs = await getAllDocs();
  return docs.map((doc) => ({ slug: doc.slug }));
}

export async function generateMetadata({
  params,
}: {
  params: Promise<{ slug: string }>;
}): Promise<Metadata> {
  const { slug } = await params;
  const doc = await getDocBySlug(slug);
  if (!doc) {
    return { title: 'Document not found — Agora' };
  }
  return {
    title: `${doc.title} — Agora Docs`,
    description: doc.description || undefined,
  };
}

export default async function DocPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const doc = await getDocBySlug(slug);
  if (!doc) notFound();

  return (
    <article className="min-w-0">
      <nav aria-label="Breadcrumb" className="mb-4 text-sm text-gray-500 dark:text-gray-400">
        <Link href="/docs" className="hover:text-indigo-600 dark:hover:text-indigo-400">
          Docs
        </Link>
        <span aria-hidden="true"> / </span>
        <span>{doc.group}</span>
        <span aria-hidden="true"> / </span>
        <span className="text-gray-700 dark:text-gray-300">{doc.title}</span>
      </nav>

      <header className="mb-6 border-b pb-5 dark:border-gray-700">
        <p className="text-xs font-semibold uppercase tracking-wide text-indigo-600 dark:text-indigo-400">
          {AUDIENCE_LABELS[doc.audience]}
        </p>
        <h1 className="mt-2 text-3xl font-bold tracking-tight">{doc.title}</h1>
      </header>

      {doc.audience === 'internal' && (
        <p className="mb-6 rounded-lg border border-amber-300 bg-amber-50 p-4 text-sm leading-6 text-amber-900 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-200">
          This is an engineering working note kept for audit. It records what was
          planned or reviewed at a point in time and is not a description of
          current behavior. For how Agora works today, read the{' '}
          <Link href="/docs/guides" className="font-semibold underline">
            task guides
          </Link>{' '}
          or the reference documentation.
        </p>
      )}

      {/* `body` is the committed markdown with its leading H1 removed — the
          header above renders the title, and printing both showed it twice. */}
      <div className="docs-prose min-w-0">
        <MarkdownRenderer content={doc.body} />
      </div>

      <div className="mt-10 flex flex-wrap items-center justify-between gap-3 border-t pt-6 text-sm text-gray-500 dark:border-gray-700 dark:text-gray-400">
        <span>
          Source:{' '}
          <code className="rounded bg-gray-100 px-1.5 py-0.5 dark:bg-gray-800">{doc.path}</code>
        </span>
        <a
          href={`${GITHUB_REPO_URL}/blob/HEAD/${doc.path}`}
          target="_blank"
          rel="noopener noreferrer"
          className="font-medium text-indigo-600 hover:underline dark:text-indigo-400"
        >
          Edit this page on GitHub
        </a>
      </div>
    </article>
  );
}
