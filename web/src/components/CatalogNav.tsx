'use client';

import Link from 'next/link';
import { usePathname } from 'next/navigation';
import {
  CONTENT_TYPES,
  contentTypeLabel,
  contentTypePath,
  isCatalogPath,
  normalizePathname,
} from '@/lib/contentTypes';

/**
 * Secondary navigation across the catalog's content types.
 *
 * Rendered from the shell so it appears on both the listing pages and the item
 * detail pages; it renders nothing outside the catalog.
 */
export function CatalogNav() {
  const pathname = normalizePathname(usePathname() ?? '');
  if (!isCatalogPath(pathname)) return null;

  return (
    <div className="border-b border-gold/15 bg-nav/80 backdrop-blur-md">
      <nav aria-label="Catalog" className="shell-wrap ui-text flex flex-wrap items-center gap-1 py-2 text-sm">
        {CONTENT_TYPES.map((type) => {
          const href = contentTypePath(type);
          const active = pathname === href || pathname.startsWith(`${href}/`);
          return (
            <Link
              key={href}
              href={href}
              aria-current={active ? 'page' : undefined}
              className={[
                'rounded-lg px-2.5 py-1 font-medium transition',
                active
                  ? 'bg-gold/12 font-semibold text-gold-bright'
                  : 'text-ink-muted hover:bg-gold/8 hover:text-gold-bright',
              ].join(' ')}
            >
              {contentTypeLabel(type)}
            </Link>
          );
        })}
      </nav>
    </div>
  );
}
