import Link from 'next/link';
import { getAllItems, CONTENT_TYPES, contentTypeLabel, contentTypePath } from '@/lib/db';
import { GITHUB_REPO_URL, GITHUB_RELEASES_URL, SPONSORS_URL } from '@/lib/site';
import { DownloadButton } from '@/components/DownloadButton';
import { HeroTrailer } from '@/components/HeroTrailer';

export default async function HomePage() {
  const allItems = await getAllItems();
  const mods = allItems.filter((i) => i.content_type === 'mod');
  const packs = allItems.filter((i) => i.content_type === 'pack');
  const featuredMods = mods.slice(0, 4);
  const featuredPacks = packs.slice(0, 2);

  const countsByType: Record<string, number> = {};
  for (const item of allItems) {
    countsByType[item.content_type] = (countsByType[item.content_type] ?? 0) + 1;
  }

  return (
    <div className="space-y-14">
      {/* ── Hero ─────────────────────────────────────────────────────────
          Sits over the living world; the panel keeps the copy legible while
          the world stays visible in the gutters beside it. */}
      <section className="panel relative overflow-hidden text-center">
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-gold to-transparent"
        />
        <p className="eyebrow">Community-curated &middot; Ad-free &middot; Open source</p>
        <h1 className="mt-3 bg-gold-sheen bg-clip-text text-4xl font-extrabold tracking-tight text-transparent md:text-5xl">
          Agora Minecraft Mod Launcher
        </h1>
        <p className="mx-auto mt-4 max-w-2xl text-lg leading-8 text-ink-muted">
          A bespoke, boutique, community-curated Minecraft mod platform. Curated, not warehoused &mdash;
          every entry hand-picked and community reviewed.
        </p>

        <HeroTrailer />

        <div className="mt-9 flex flex-wrap justify-center gap-3">
          <Link href="/mods" className="btn-gold px-5 py-3">
            Browse the database
          </Link>
          <Link href="/docs" className="btn-ghost px-5 py-3">
            Read the docs
          </Link>
          <a
            href={GITHUB_REPO_URL}
            className="btn-ghost px-5 py-3"
            target="_blank"
            rel="noopener noreferrer"
          >
            View on GitHub
          </a>
        </div>
        <div className="mt-6">
          <DownloadButton />
        </div>
      </section>

      {/* ── Value proposition ────────────────────────────────────────── */}
      <section className="panel">
        <div className="flex flex-col gap-6 md:flex-row md:items-start md:justify-between">
          <div className="max-w-2xl">
            <h2 className="rule-gold text-2xl font-bold text-ink">More than a catalog</h2>
            <p className="mt-4 leading-8 text-ink-muted">
              Agora keeps Minecraft setups isolated, resolves dependency-aware install plans,
              checks health before launch, recommends compatible loader versions, and creates
              recovery points before risky changes.
            </p>
            <Link href="/about" className="ui-text mt-3 inline-flex text-sm font-semibold text-gold hover:underline">
              What makes Agora special →
            </Link>
          </div>
          <div className="flex shrink-0 flex-wrap gap-3 md:pt-2">
            <Link href="/docs" className="btn-gold px-4 py-2.5 text-sm">
              First-run guide
            </Link>
            <a
              href={GITHUB_RELEASES_URL}
              className="btn-ghost px-4 py-2.5 text-sm"
              target="_blank"
              rel="noopener noreferrer"
            >
              Releases
            </a>
          </div>
        </div>
      </section>

      {/* ── Sponsor ──────────────────────────────────────────────────── */}
      <section className="panel border-pink-800/60 bg-[linear-gradient(135deg,rgba(184,69,106,0.14),rgba(23,38,59,0.93)_55%)]">
        <div className="flex flex-col gap-5 md:flex-row md:items-center md:justify-between">
          <div className="max-w-2xl">
            <h2 className="flex items-center gap-2.5 text-xl font-bold text-ink">
              <span aria-hidden="true" className="text-pink-400">&#9829;</span>
              Keep Agora free &amp; thriving
            </h2>
            <p className="mt-3 leading-8 text-ink-muted">
              Agora is free, open source, and ad-free &mdash; funded only by community donations. If you
              enjoy using Agora, please consider sponsoring its development. Your support keeps Agora
              improving and helps fund awesome new projects for the community. Thank you!
            </p>
          </div>
          <div className="flex shrink-0 flex-wrap gap-3">
            <a
              href={SPONSORS_URL}
              target="_blank"
              rel="noopener noreferrer"
              className="ui-text rounded-lg bg-pink-600 px-5 py-3 font-semibold text-white shadow-md transition hover:bg-pink-500"
            >
              Sponsor on GitHub &#9829;
            </a>
            <Link href="/about" className="btn-ghost px-5 py-3">
              Learn more
            </Link>
          </div>
        </div>
      </section>

      {/* ── Browse by type ───────────────────────────────────────────── */}
      <section className="panel">
        <h2 className="rule-gold mb-7 text-2xl font-bold text-ink">Browse by type</h2>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          {CONTENT_TYPES.map((type) => {
            const count = countsByType[type] ?? 0;
            return (
              <Link
                key={type}
                href={contentTypePath(type as any)}
                className="panel-inset card-lift group block p-5"
              >
                <h3 className="text-lg font-semibold text-ink transition group-hover:text-gold-bright">
                  {contentTypeLabel(type as any)}
                </h3>
                <p className="ui-text mt-1.5 text-sm text-ink-muted/75">
                  <span className="font-semibold text-gold">{count}</span> curated {type} entries
                </p>
              </Link>
            );
          })}
        </div>
      </section>

      {/* ── Registry growth notice ───────────────────────────────────── */}
      {allItems.length < 20 && (
        <section className="panel border-amber-700/60 bg-[linear-gradient(135deg,rgba(154,101,28,0.16),rgba(23,38,59,0.93)_55%)]">
          <h2 className="text-lg font-semibold text-amber-200">The registry is growing</h2>
          <p className="mt-2.5 leading-7 text-ink-muted">
            Agora currently contains <span className="font-semibold text-gold-bright">{allItems.length}</span>{' '}
            curated items. The registry grows through community review and contribution.
          </p>
          <div className="ui-text mt-4 flex flex-wrap gap-5 text-sm">
            <a
              href={`${GITHUB_REPO_URL}/blob/HEAD/REGISTRY_CURATION_REFERENCE.md`}
              target="_blank"
              rel="noopener noreferrer"
              className="font-semibold text-gold-bright underline underline-offset-4 hover:text-ink"
            >
              Contribution guide
            </a>
            <a
              href={`${GITHUB_REPO_URL}/issues/new?template=propose-mod.yml`}
              target="_blank"
              rel="noopener noreferrer"
              className="font-semibold text-gold-bright underline underline-offset-4 hover:text-ink"
            >
              Propose a project
            </a>
          </div>
        </section>
      )}

      {/* ── Featured ─────────────────────────────────────────────────── */}
      {featuredPacks.length > 0 && (
        <section className="panel">
          <h2 className="rule-gold mb-7 text-2xl font-bold text-ink">Featured modpacks</h2>
          <div className="grid gap-4 sm:grid-cols-2">
            {featuredPacks.map((pack) => (
              <Link
                key={pack.id}
                href={`/packs/${pack.id}`}
                className="panel-inset card-lift group block p-5"
              >
                <h3 className="text-lg font-semibold text-ink transition group-hover:text-gold-bright">
                  {pack.name}
                </h3>
                <p className="mt-2 line-clamp-2 text-sm leading-6 text-ink-muted/80">
                  {pack.curator_note}
                </p>
              </Link>
            ))}
          </div>
        </section>
      )}

      {featuredMods.length > 0 && (
        <section className="panel">
          <h2 className="rule-gold mb-7 text-2xl font-bold text-ink">Top mods</h2>
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            {featuredMods.map((mod) => (
              <Link
                key={mod.id}
                href={`/mods/${mod.id}`}
                className="panel-inset card-lift group block p-5"
              >
                <h3 className="font-semibold text-ink transition group-hover:text-gold-bright">
                  {mod.name}
                </h3>
                <p className="mt-2 line-clamp-3 text-sm leading-6 text-ink-muted/80">
                  {mod.curator_note}
                </p>
              </Link>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
