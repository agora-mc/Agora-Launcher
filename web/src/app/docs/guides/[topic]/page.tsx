import type { Metadata } from 'next';
import Link from 'next/link';
import { notFound } from 'next/navigation';
import { GuideBody } from '@/components/GuideBody';
import { getGuideNeighbors, getGuideTopic, getGuideTopicIds } from '@/lib/guide';

export async function generateStaticParams() {
  return getGuideTopicIds().map((topic) => ({ topic }));
}

export async function generateMetadata({
  params,
}: {
  params: Promise<{ topic: string }>;
}): Promise<Metadata> {
  const { topic: id } = await params;
  const topic = getGuideTopic(id);
  if (!topic) return { title: 'Guide not found — Agora' };
  return {
    title: `${topic.title} — Agora Guides`,
    description: topic.description,
  };
}

export default async function GuideTopicPage({
  params,
}: {
  params: Promise<{ topic: string }>;
}) {
  const { topic: id } = await params;
  const topic = getGuideTopic(id);
  if (!topic) notFound();

  const { prev, next } = getGuideNeighbors(id);

  return (
    <article className="min-w-0">
      <nav aria-label="Breadcrumb" className="mb-4 text-sm text-gray-500 dark:text-gray-400">
        <Link href="/docs" className="hover:text-indigo-600 dark:hover:text-indigo-400">
          Docs
        </Link>
        <span aria-hidden="true"> / </span>
        <Link href="/docs/guides" className="hover:text-indigo-600 dark:hover:text-indigo-400">
          Task guides
        </Link>
        <span aria-hidden="true"> / </span>
        <span className="text-gray-700 dark:text-gray-300">{topic.shortTitle}</span>
      </nav>

      <header className="mb-6 border-b pb-5 dark:border-gray-700">
        <p className="text-xs font-semibold uppercase tracking-wide text-indigo-600 dark:text-indigo-400">
          {topic.category}
        </p>
        <h1 className="mt-2 text-3xl font-bold tracking-tight">{topic.title}</h1>
        <p className="mt-3 max-w-3xl text-gray-600 dark:text-gray-300">{topic.description}</p>
        <p className="mt-4 text-sm text-gray-500 dark:text-gray-400">
          Two versions of this page follow.{' '}
          <a href="#basic" className="font-medium text-indigo-600 hover:underline dark:text-indigo-400">
            Step by step
          </a>{' '}
          walks through the task in order.{' '}
          <a
            href="#advanced"
            className="font-medium text-indigo-600 hover:underline dark:text-indigo-400"
          >
            In depth
          </a>{' '}
          explains the underlying behavior. They describe the same product, at
          different levels of detail.
        </p>
      </header>

      <div className="space-y-12">
        <GuideBody page={topic.basic} level="basic" />
        <GuideBody page={topic.advanced} level="advanced" />
      </div>

      <div className="mt-12 border-t pt-6 dark:border-gray-700">
        <p className="text-sm text-gray-500 dark:text-gray-400">
          This guide is the same text Agora shows in its built-in{' '}
          <strong>Help &amp; Guide</strong>, which is searchable and links directly to
          the screens it describes.
        </p>
        <nav aria-label="Guide navigation" className="mt-4 flex flex-wrap justify-between gap-4 text-sm">
          {prev ? (
            <Link
              href={`/docs/guides/${prev.id}`}
              className="font-medium text-indigo-600 hover:underline dark:text-indigo-400"
            >
              ← {prev.shortTitle}
            </Link>
          ) : (
            <span />
          )}
          {next && (
            <Link
              href={`/docs/guides/${next.id}`}
              className="font-medium text-indigo-600 hover:underline dark:text-indigo-400"
            >
              {next.shortTitle} →
            </Link>
          )}
        </nav>
      </div>
    </article>
  );
}
