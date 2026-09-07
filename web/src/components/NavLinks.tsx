'use client';

import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { normalizePathname } from '@/lib/contentTypes';

export interface NavItem {
  href: string;
  label: string;
  /** 'exact' matches only the exact pathname; 'prefix' also matches sub-paths. */
  match: 'exact' | 'prefix';
  /**
   * Extra paths that also mark this link active, matched as prefixes. Used by
   * the Catalog link, which points at one content type but owns them all.
   */
  activePaths?: string[];
}

export function NavLinks({ items }: { items: NavItem[] }) {
  const pathname = normalizePathname(usePathname() ?? '');

  const matchesPrefix = (base: string) =>
    pathname === base || pathname.startsWith(`${base}/`);

  const isActive = (item: NavItem) =>
    (item.match === 'exact' ? pathname === item.href : matchesPrefix(item.href)) ||
    (item.activePaths ?? []).some(matchesPrefix);

  return (
    <nav aria-label="Primary" className="ui-text flex flex-wrap items-center gap-x-1 gap-y-1 text-sm">
      {items.map((item) => {
        const active = isActive(item);
        return (
          <Link
            key={item.href}
            href={item.href}
            aria-current={active ? 'page' : undefined}
            className={[
              'relative rounded-lg px-2.5 py-1.5 font-medium transition',
              active
                ? 'bg-gold/12 font-semibold text-gold-bright'
                : 'text-ink-muted hover:bg-gold/8 hover:text-gold-bright',
            ].join(' ')}
          >
            {item.label}
            {active && (
              <span
                aria-hidden="true"
                className="absolute inset-x-2.5 -bottom-0.5 h-px rounded-full bg-gold-bright"
              />
            )}
          </Link>
        );
      })}
    </nav>
  );
}
