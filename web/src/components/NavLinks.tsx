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
    <nav aria-label="Primary" className="flex flex-wrap gap-4 text-sm">
      {items.map((item) => {
        const active = isActive(item);
        return (
          <Link
            key={item.href}
            href={item.href}
            aria-current={active ? 'page' : undefined}
            className={
              active
                ? 'font-semibold text-indigo-600 underline underline-offset-4 dark:text-indigo-400'
                : 'hover:text-indigo-600 dark:hover:text-indigo-400'
            }
          >
            {item.label}
          </Link>
        );
      })}
    </nav>
  );
}
