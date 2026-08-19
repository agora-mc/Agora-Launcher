import Link from 'next/link';
import { GITHUB_REPO_URL, GITHUB_RELEASES_URL, SPONSORS_URL } from '@/lib/site';
import { NavLinks, type NavItem } from './NavLinks';
import { FontScaleControl } from './FontScaleControl';

export function Shell({ children }: { children: React.ReactNode }) {
  const navItems: NavItem[] = [
    { href: '/', label: 'Home', match: 'exact' },
    { href: '/docs', label: 'Docs', match: 'prefix' },
    { href: '/governance', label: 'Governance', match: 'prefix' },
    { href: '/about', label: 'The Agora Difference', match: 'prefix' },
    { href: '/credits', label: "Agora's Credits", match: 'exact' },
  ];

  return (
    <div className="flex min-h-screen flex-col bg-gray-50 text-gray-900 dark:bg-gray-900 dark:text-gray-100">
      <a
        href="#main-content"
        className="sr-only z-50 rounded bg-white px-4 py-2 text-gray-900 focus:not-sr-only focus:fixed focus:left-4 focus:top-4"
      >
        Skip to content
      </a>
      <header className="border-b bg-white dark:border-gray-700 dark:bg-gray-800">
        <div className="mx-auto flex max-w-6xl flex-col gap-4 px-6 py-4 md:flex-row md:items-center md:justify-between">
          <Link href="/" className="text-xl font-bold">
            Agora
          </Link>
          <div className="flex flex-col gap-3 md:flex-row md:items-center md:gap-6">
            <NavLinks items={navItems} />
            <FontScaleControl />
          </div>
        </div>
      </header>

      <main id="main-content" className="mx-auto w-full max-w-6xl flex-1 px-6 py-8">{children}</main>

      <footer className="border-t bg-white px-6 py-6 text-sm text-gray-600 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-400">
        <div className="mx-auto flex max-w-6xl flex-col gap-4 md:flex-row md:items-center md:justify-between">
          <p>Agora — a bespoke, boutique, community-curated Minecraft mod launcher.</p>
          <div className="flex flex-wrap gap-4">
            <Link href="/docs" className="hover:text-indigo-600 dark:hover:text-indigo-400">
              Docs
            </Link>
            <Link href="/governance" className="hover:text-indigo-600 dark:hover:text-indigo-400">
              Governance
            </Link>
            <a
              href={GITHUB_REPO_URL}
              className="hover:text-indigo-600 dark:hover:text-indigo-400"
              target="_blank"
              rel="noopener noreferrer"
            >
              GitHub
            </a>
            <a
              href={GITHUB_RELEASES_URL}
              className="hover:text-indigo-600 dark:hover:text-indigo-400"
              target="_blank"
              rel="noopener noreferrer"
            >
              Download Desktop
            </a>
            <a
              href={SPONSORS_URL}
              className="inline-flex items-center gap-1 font-semibold text-pink-600 hover:text-pink-700 dark:text-pink-400 dark:hover:text-pink-300"
              target="_blank"
              rel="noopener noreferrer"
            >
              <span aria-hidden="true">♥</span> Donate
            </a>
          </div>
        </div>
        <div className="mx-auto mt-4 max-w-6xl border-t border-gray-100 pt-4 text-xs leading-5 dark:border-gray-700">
          <p>
            Agora is free, ad-free, and open source — funded by community donations, not ads or data.
            If Agora helps you discover great mods,{' '}
            <a href={SPONSORS_URL} target="_blank" rel="noopener noreferrer" className="font-medium text-pink-600 hover:underline dark:text-pink-400">
              consider sponsoring on GitHub
            </a>{' '}
            to keep it improving and to support awesome new projects in the future. Every contribution helps — thank you!
          </p>
        </div>
      </footer>
    </div>
  );
}
