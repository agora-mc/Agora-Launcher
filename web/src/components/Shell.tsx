import Image from 'next/image';
import Link from 'next/link';
import { GITHUB_REPO_URL, GITHUB_RELEASES_URL, SPONSORS_URL } from '@/lib/site';
import { NavLinks, type NavItem } from './NavLinks';
import { FontScaleControl } from './FontScaleControl';
import LivingBackgroundMount from './LivingBackgroundMount';

export function Shell({ children }: { children: React.ReactNode }) {
  const navItems: NavItem[] = [
    { href: '/', label: 'Home', match: 'exact' },
    { href: '/docs', label: 'Docs', match: 'prefix' },
    { href: '/governance', label: 'Governance', match: 'prefix' },
    { href: '/about', label: 'The Agora Difference', match: 'prefix' },
    { href: '/credits', label: "Agora's Credits", match: 'exact' },
  ];

  return (
    <>
      {/* The living world, behind everything. Client-only; renders nothing
          during the static export and under prefers-reduced-motion. */}
      <LivingBackgroundMount />

      <div className="app-shell flex min-h-screen flex-col">
        <a
          href="#main-content"
          className="sr-only z-50 rounded-lg bg-gold px-4 py-2 font-semibold text-canvas focus:not-sr-only focus:fixed focus:left-4 focus:top-4"
        >
          Skip to content
        </a>

        <header className="sticky top-0 z-30 border-b border-gold/20 bg-nav/95 backdrop-blur-md">
          <div className="shell-wrap flex flex-col gap-4 py-3.5 md:flex-row md:items-center md:justify-between">
            <Link href="/" className="group flex items-center gap-3">
              <Image
                src="/agora-logo.png"
                alt=""
                width={40}
                height={40}
                priority
                className="h-10 w-10 rounded-lg shadow-md ring-1 ring-gold/30 transition group-hover:ring-gold/70"
              />
              <span className="flex flex-col leading-none">
                <span className="font-display text-2xl font-bold tracking-tight text-ink">Agora</span>
                <span className="ui-text mt-1 text-[0.62rem] font-semibold uppercase tracking-[0.18em] text-gold">
                  Mod Launcher
                </span>
              </span>
            </Link>
            <div className="flex flex-col gap-3 md:flex-row md:items-center md:gap-6">
              <NavLinks items={navItems} />
              <FontScaleControl />
            </div>
          </div>
        </header>

        <main id="main-content" className="shell-wrap flex-1 py-10">
          {children}
        </main>

        <footer className="mt-8 border-t border-gold/20 bg-nav/95 py-8 text-sm text-ink-muted backdrop-blur-md">
          <div className="shell-wrap">
            <div className="flex flex-col gap-5 md:flex-row md:items-center md:justify-between">
              <div className="flex items-center gap-3">
                <Image
                  src="/agora-logo.png"
                  alt=""
                  width={32}
                  height={32}
                  className="h-8 w-8 rounded-md opacity-90 ring-1 ring-gold/20"
                />
                <p className="max-w-sm leading-6">
                  Agora — a bespoke, boutique, community-curated Minecraft mod launcher.
                </p>
              </div>
              <div className="ui-text flex flex-wrap gap-x-5 gap-y-2">
                <Link href="/docs" className="transition hover:text-gold-bright">
                  Docs
                </Link>
                <Link href="/governance" className="transition hover:text-gold-bright">
                  Governance
                </Link>
                <a
                  href={GITHUB_REPO_URL}
                  className="transition hover:text-gold-bright"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  GitHub
                </a>
                <a
                  href={GITHUB_RELEASES_URL}
                  className="transition hover:text-gold-bright"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  Download Desktop
                </a>
                <a
                  href={SPONSORS_URL}
                  className="inline-flex items-center gap-1 font-semibold text-pink-400 transition hover:text-pink-300"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  <span aria-hidden="true">&#9829;</span> Donate
                </a>
              </div>
            </div>

            <div className="mt-6 border-t border-gold/15 pt-5 text-xs leading-6">
              <p>
                Agora is free, ad-free, and open source &mdash; funded by community donations, not ads or data.
                If Agora helps you discover great mods,{' '}
                <a
                  href={SPONSORS_URL}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="font-semibold text-pink-400 hover:underline"
                >
                  consider sponsoring on GitHub
                </a>{' '}
                to keep it improving and to support awesome new projects in the future. Every contribution helps &mdash; thank you!
              </p>
            </div>
          </div>
        </footer>
      </div>
    </>
  );
}
