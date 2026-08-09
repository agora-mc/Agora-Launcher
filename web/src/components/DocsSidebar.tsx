import Link from 'next/link';
import type { DocPage } from '@/lib/docs';

interface DocsSidebarProps {
  docs: DocPage[];
  activeSlug?: string;
}

/** Sidebar listing every documentation page, with the active page marked. */
export function DocsSidebar({ docs, activeSlug }: DocsSidebarProps) {
  return (
    <nav aria-label="Documentation" className="shrink-0 lg:sticky lg:top-8 lg:max-h-[calc(100vh-4rem)] lg:w-60 lg:overflow-y-auto">
      <p className="px-2 text-xs font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
        Documentation
      </p>
      <ul className="mt-2 space-y-0.5 text-sm">
        {docs.map((doc) => {
          const active = doc.slug === activeSlug;
          return (
            <li key={doc.slug}>
              <Link
                href={`/docs/${doc.slug}`}
                aria-current={active ? 'page' : undefined}
                className={
                  active
                    ? 'block rounded-lg bg-indigo-100 px-2 py-1.5 font-semibold text-indigo-700 dark:bg-indigo-900 dark:text-indigo-200'
                    : 'block rounded-lg px-2 py-1.5 text-gray-700 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-800'
                }
              >
                {doc.title}
              </Link>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
