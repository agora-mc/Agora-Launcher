import Link from 'next/link';
import { getGuideCategories, getGuideTopics } from '@/lib/guide';

export const metadata = {
  title: 'Task guides — Agora Docs',
  description:
    'Step-by-step and in-depth guides for every part of Agora, mirrored from the app’s built-in Help & Guide.',
};

export default function GuidesIndexPage() {
  const categories = getGuideCategories();
  const total = getGuideTopics().length;

  return (
    <div className="min-w-0 space-y-10">
      <header>
        <nav aria-label="Breadcrumb" className="mb-4 text-sm text-gray-500 dark:text-gray-400">
          <Link href="/docs" className="hover:text-indigo-600 dark:hover:text-indigo-400">
            Docs
          </Link>
          <span aria-hidden="true"> / </span>
          <span className="text-gray-700 dark:text-gray-300">Task guides</span>
        </nav>
        <h1 className="text-3xl font-bold tracking-tight">Task guides</h1>
        <p className="mt-3 max-w-3xl leading-7 text-gray-600 dark:text-gray-300">
          {total} guides covering what Agora does and how to do it. Each one is
          written twice: <strong>step by step</strong> for following along, and{' '}
          <strong>in depth</strong> for understanding the behavior behind the
          steps. Pick whichever suits you — they describe the same product.
        </p>
        <p className="mt-3 max-w-3xl text-sm text-gray-500 dark:text-gray-400">
          These are the same guides built into the desktop app under{' '}
          <strong>Help &amp; Guide</strong>, published here so you can read them
          before installing or link someone straight to an answer.
        </p>
      </header>

      {categories.map((section) => (
        <section key={section.category}>
          <h2 className="text-2xl font-bold">{section.category}</h2>
          <p className="mt-1 text-gray-600 dark:text-gray-300">{section.blurb}</p>
          <div className="mt-4 grid gap-4 sm:grid-cols-2">
            {section.topics.map((topic) => (
              <Link
                key={topic.id}
                href={`/docs/guides/${topic.id}`}
                className="panel-inset card-lift p-5"
              >
                <h3 className="font-semibold text-indigo-700 dark:text-indigo-400">
                  {topic.title}
                </h3>
                <p className="mt-1 text-sm leading-6 text-gray-600 dark:text-gray-300">
                  {topic.description}
                </p>
              </Link>
            ))}
          </div>
        </section>
      ))}

      <section className="panel-inset p-6">
        <h2 className="text-xl font-bold">Looking for exact syntax instead?</h2>
        <p className="mt-2 leading-7 text-gray-600 dark:text-gray-300">
          Guides explain workflows. For command flags, exit codes, file
          locations, and evidence collection, use the reference documentation.
        </p>
        <div className="mt-4 flex flex-wrap gap-4 text-sm font-medium">
          <Link href="/docs/cli" className="text-indigo-600 hover:underline dark:text-indigo-400">
            CLI reference
          </Link>
          <Link
            href="/docs/troubleshooting"
            className="text-indigo-600 hover:underline dark:text-indigo-400"
          >
            Troubleshooting
          </Link>
          <Link href="/docs/support" className="text-indigo-600 hover:underline dark:text-indigo-400">
            Data, logs, and support evidence
          </Link>
        </div>
      </section>
    </div>
  );
}
