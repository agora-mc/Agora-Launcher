'use client';

import Link from 'next/link';
import { usePathname } from 'next/navigation';

export interface NavItem {
  href: string;
  label: string;
  /** 'exact' matches only the exact pathname; 'prefix' also matches sub-paths. */
  match: 'exact' | 'prefix';
}

export function NavLinks({ items }: { items: NavItem[] }) {
  const pathname = usePathname() ?? '';

  const isActive = (item: NavItem) =>
    item.match === 'exact'
      ? pathname === item.href
      : pathname === item.href || pathname.startsWith(`${item.href}/`);

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
