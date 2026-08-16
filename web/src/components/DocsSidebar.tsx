'use client';

import { useEffect, useState } from 'react';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import type { DocNavSection } from '@/lib/docs';

interface DocsSidebarProps {
  sections: DocNavSection[];
}

const START_HERE = [
  { href: '/docs', label: 'Documentation home' },
  { href: '/docs/install', label: 'Install Agora' },
  { href: '/docs/tour', label: 'Take a tour' },
  { href: '/docs/guides', label: 'Task guides' },
];

const linkClass = (active: boolean) =>
  active
    ? 'block rounded-lg bg-indigo-100 px-2 py-1.5 font-semibold text-indigo-700 dark:bg-indigo-900 dark:text-indigo-200'
    : 'block rounded-lg px-2 py-1.5 text-gray-700 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-800';

function NavGroup({
  group,
  pathname,
}: {
  group: DocNavSection['groups'][number];
  pathname: string;
}) {
  return (
    <div className="mt-3">
      <p className="px-2 text-xs font-medium text-gray-400 dark:text-gray-500">{group.group}</p>
      <ul className="mt-1 space-y-0.5 text-sm">
        {group.links.map((link) => {
          const href = `/docs/${link.slug}`;
          const active = pathname === href;
          return (
            <li key={link.slug}>
              <Link
                href={href}
                aria-current={active ? 'page' : undefined}
                className={linkClass(active)}
              >
                {link.title}
              </Link>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/**
 * Documentation navigation, grouped by audience so a player is never asked to
 * scan past maintainer references to reach troubleshooting. Internal working
 * notes stay published — cross-references to them must resolve — but they are
 * collapsed out of the way and labelled as non-authoritative.
 */
export function DocsSidebar({ sections }: DocsSidebarProps) {
  const pathname = usePathname() ?? '';
  // Narrow screens stack the sidebar above the article, so leaving it expanded
  // would bury every page under ~20 links. It is a disclosure below `lg` and a
  // plain sticky column above it.
  const [mobileOpen, setMobileOpen] = useState(false);
  useEffect(() => setMobileOpen(false), [pathname]);

  const primary = sections.filter((section) => section.audience !== 'internal');
  const internal = sections.filter((section) => section.audience === 'internal');
  const internalIsActive = internal.some((section) =>
    section.groups.some((group) =>
      group.links.some((link) => pathname === `/docs/${link.slug}`)
    )
  );

  return (
    <nav
      aria-label="Documentation"
      className="shrink-0 lg:sticky lg:top-8 lg:max-h-[calc(100vh-4rem)] lg:w-64 lg:overflow-y-auto"
    >
      <button
        type="button"
        onClick={() => setMobileOpen((open) => !open)}
        aria-expanded={mobileOpen}
        aria-controls="docs-nav-panel"
        className="flex w-full items-center justify-between rounded-lg border px-4 py-3 text-sm font-semibold dark:border-gray-700 lg:hidden"
      >
        Browse documentation
        <span aria-hidden="true">{mobileOpen ? '▲' : '▼'}</span>
      </button>

      <div id="docs-nav-panel" className={`${mobileOpen ? 'mt-4' : 'hidden'} lg:mt-0 lg:block`}>
      <p className="px-2 text-xs font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
        Start here
      </p>
      <ul className="mt-2 space-y-0.5 text-sm">
        {START_HERE.map((item) => {
          const active =
            item.href === '/docs'
              ? pathname === '/docs'
              : pathname === item.href || pathname.startsWith(`${item.href}/`);
          return (
            <li key={item.href}>
              <Link
                href={item.href}
                aria-current={active ? 'page' : undefined}
                className={linkClass(active)}
              >
                {item.label}
              </Link>
            </li>
          );
        })}
      </ul>

      {primary.map((section) => (
        <div key={section.audience} className="mt-6">
          <p className="px-2 text-xs font-semibold uppercase tracking-wide text-gray-500 dark:text-gray-400">
            {section.label}
          </p>
          {section.groups.map((group) => (
            <NavGroup key={group.group} group={group} pathname={pathname} />
          ))}
        </div>
      ))}

      {internal.length > 0 && (
        <details className="mt-6" open={internalIsActive}>
          <summary className="cursor-pointer px-2 text-xs font-semibold uppercase tracking-wide text-gray-500 hover:text-indigo-600 dark:text-gray-400 dark:hover:text-indigo-400">
            Working notes and archive
          </summary>
          <p className="px-2 pt-2 text-xs leading-5 text-gray-500 dark:text-gray-400">
            Engineering records kept for audit. They are not current product
            documentation.
          </p>
          {internal.map((section) =>
            section.groups.map((group) => (
              <NavGroup key={group.group} group={group} pathname={pathname} />
            ))
          )}
        </details>
      )}
      </div>
    </nav>
  );
}
