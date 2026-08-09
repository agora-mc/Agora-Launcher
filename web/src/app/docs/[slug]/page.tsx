import type { Metadata } from 'next';
import Link from 'next/link';
import { notFound } from 'next/navigation';
import MarkdownRenderer from '@/components/MarkdownRenderer';
import { DocsSidebar } from '@/components/DocsSidebar';
import { getAllDocs, getDocBySlug } from '@/lib/docs';

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
  const [doc, allDocs] = await Promise.all([getDocBySlug(slug), getAllDocs()]);
  if (!doc) notFound();

  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-8 lg:flex-row">
      <DocsSidebar docs={allDocs} activeSlug={doc.slug} />

      <article className="min-w-0 flex-1">
        <nav aria-label="Breadcrumb" className="mb-4 text-sm text-gray-500 dark:text-gray-400">
          <Link href="/docs" className="hover:text-indigo-600 dark:hover:text-indigo-400">
            Docs
          </Link>
          <span aria-hidden="true"> / </span>
          <span className="text-gray-700 dark:text-gray-300">{doc.title}</span>
        </nav>

        <header className="mb-6 border-b pb-5 dark:border-gray-700">
          <h1 className="text-3xl font-bold tracking-tight">{doc.title}</h1>
          {doc.description && (
            <p className="mt-3 max-w-3xl text-gray-600 dark:text-gray-300">{doc.description}</p>
          )}
        </header>

        <div className="docs-prose min-w-0">
          <MarkdownRenderer content={doc.content} />
        </div>

        <div className="mt-10 border-t pt-6 text-sm text-gray-500 dark:border-gray-700 dark:text-gray-400">
          Source: <code className="rounded bg-gray-100 px-1.5 py-0.5 dark:bg-gray-800">{doc.path}</code>
        </div>
      </article>
    </div>
  );
}
