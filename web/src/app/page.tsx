import Link from 'next/link';
import { getAllItems, CONTENT_TYPES, contentTypeLabel, contentTypePath } from '@/lib/db';
import { GITHUB_REPO_URL, GITHUB_RELEASES_URL } from '@/lib/site';
import { DownloadButton } from '@/components/DownloadButton';

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
    <div className="space-y-12">
      <section className="rounded-2xl bg-indigo-600 px-6 py-16 text-center text-white dark:bg-indigo-700">
        <h1 className="text-4xl font-extrabold tracking-tight md:text-5xl">
          Agora Minecraft Mod Launcher
        </h1>
        <p className="mx-auto mt-4 max-w-2xl text-lg text-indigo-100">
          A boutique, community-curated Minecraft mod platform.
        </p>
        <div className="mt-8 flex flex-wrap justify-center gap-4">
          <Link
            href="/mods"
            className="rounded-lg bg-white px-5 py-3 font-semibold text-indigo-700 shadow-sm hover:bg-gray-100"
          >
            Browse the database
          </Link>
          <a
            href={GITHUB_REPO_URL}
            className="rounded-lg border border-white px-5 py-3 font-semibold text-white hover:bg-white/10"
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

      <section>
        <h2 className="mb-6 text-2xl font-bold">Browse by type</h2>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          {CONTENT_TYPES.map((type) => (
            <Link
              key={type}
              href={contentTypePath(type as any)}
              className="rounded-xl border bg-white p-5 shadow-sm transition hover:shadow-md dark:border-gray-700 dark:bg-gray-800"
            >
              <h3 className="text-lg font-semibold">{contentTypeLabel(type as any)}</h3>
              <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                {countsByType[type] ?? 0} curated {type} entries
              </p>
            </Link>
          ))}
        </div>
      </section>

      {allItems.length < 20 && (
        <section className="rounded-xl border border-amber-200 bg-amber-50 p-6 dark:border-amber-800 dark:bg-amber-950">
          <h2 className="text-lg font-semibold text-amber-900 dark:text-amber-200">
            The registry is growing
          </h2>
          <p className="mt-2 text-sm text-amber-800 dark:text-amber-300">
            Agora currently contains {allItems.length} curated items. The registry grows through
            community review and contribution.
          </p>
          <div className="mt-4 flex flex-wrap gap-3 text-sm">
            <a
              href={`${GITHUB_REPO_URL}/blob/main/REGISTRY_CURATION_REFERENCE.md`}
              target="_blank"
              rel="noopener noreferrer"
              className="text-amber-700 underline hover:text-amber-900 dark:text-amber-400"
            >
              Contribution guide
            </a>
            <a
              href={`${GITHUB_REPO_URL}/issues/new?template=propose-mod.yml`}
              target="_blank"
              rel="noopener noreferrer"
              className="text-amber-700 underline hover:text-amber-900 dark:text-amber-400"
            >
              Propose a project
            </a>
          </div>
        </section>
      )}

      {featuredPacks.length > 0 && (
        <section>
          <h2 className="mb-6 text-2xl font-bold">Featured modpacks</h2>
          <div className="grid gap-4 sm:grid-cols-2">
            {featuredPacks.map((pack) => (
              <Link
                key={pack.id}
                href={`/packs/${pack.id}`}
                className="rounded-xl border bg-white p-5 shadow-sm transition hover:shadow-md dark:border-gray-700 dark:bg-gray-800"
              >
                <h3 className="text-lg font-semibold">{pack.name}</h3>
                <p className="mt-2 line-clamp-2 text-sm text-gray-600 dark:text-gray-400">
                  {pack.curator_note}
                </p>
              </Link>
            ))}
          </div>
        </section>
      )}

      {featuredMods.length > 0 && (
        <section>
          <h2 className="mb-6 text-2xl font-bold">Top mods</h2>
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            {featuredMods.map((mod) => (
              <Link
                key={mod.id}
                href={`/mods/${mod.id}`}
                className="rounded-xl border bg-white p-5 shadow-sm transition hover:shadow-md dark:border-gray-700 dark:bg-gray-800"
              >
                <h3 className="font-semibold">{mod.name}</h3>
                <p className="mt-2 line-clamp-3 text-sm text-gray-600 dark:text-gray-400">
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
