'use client';

import { useState, useMemo } from 'react';
import Link from 'next/link';
import type { RegistryItem } from '@/lib/db';

interface CatalogProps {
  items: RegistryItem[];
  typeLabel: string;
  typePath: string;
}

function ItemIcon({ item }: { item: RegistryItem }) {
  if (!item.icon_url) return null;
  try {
    const url = new URL(item.icon_url);
    if (url.protocol !== 'https:') return null;
  } catch {
    return null;
  }
  return (
    // eslint-disable-next-line @next/next/no-img-element
    <img
      src={item.icon_url}
      alt=""
      className="h-12 w-12 shrink-0 rounded-lg object-contain"
      loading="lazy"
    />
  );
}

export function Catalog({ items, typeLabel, typePath }: CatalogProps) {
  const [query, setQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [selectedMcVersion, setSelectedMcVersion] = useState<string>('all');
  const [selectedLoader, setSelectedLoader] = useState<string>('all');
  const [selectedSort, setSelectedSort] = useState<'net_score' | 'velocity' | 'newest'>('net_score');

  const categories = useMemo(() => {
    const cats = new Set<string>();
    items.forEach((item) => item.categories.forEach((c) => cats.add(c)));
    return Array.from(cats).sort();
  }, [items]);

  const filtered = useMemo(() => {
    let result = items.slice();

    if (selectedCategory) {
      result = result.filter((item) => item.categories.includes(selectedCategory));
    }

    if (selectedMcVersion !== 'all') {
      result = result.filter((item) =>
        item.compatible_versions.some((v) => v.mc_version === selectedMcVersion)
      );
    }

    if (selectedLoader !== 'all') {
      result = result.filter((item) =>
        item.compatible_versions.some((v) => v.loader === selectedLoader)
      );
    }

    const q = query.toLowerCase().trim();
    if (q) {
      result = result.filter((item) => {
        const text = `${item.name} ${item.curator_note} ${item.categories.join(' ')} ${item.author ?? ''}`.toLowerCase();
        return text.includes(q);
      });
    }

    switch (selectedSort) {
      case 'velocity':
        result.sort((a, b) => b.velocity - a.velocity);
        break;
      case 'newest':
        result.sort((a, b) => (b.date_added ?? '').localeCompare(a.date_added ?? ''));
        break;
      case 'net_score':
      default:
        result.sort((a, b) => b.net_score - a.net_score);
        break;
    }

    return result;
  }, [items, selectedCategory, selectedMcVersion, selectedLoader, selectedSort, query]);

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-3xl font-bold">{typeLabel}</h1>
        <p className="text-gray-600 dark:text-gray-400">
          {filtered.length} curated {typeLabel.toLowerCase()}.
        </p>
      </div>

      <div className="flex flex-col sm:flex-row gap-3">
        <div className="flex flex-wrap gap-2">
          <button
            onClick={() => setSelectedCategory(null)}
            className={`rounded-full px-3 py-1 text-sm font-medium transition ${
              selectedCategory === null
                ? 'bg-indigo-600 text-white'
                : 'bg-gray-100 text-gray-700 hover:bg-gray-200 dark:bg-gray-700 dark:text-gray-300 dark:hover:bg-gray-600'
            }`}
          >
            All
          </button>
          {categories.map((cat) => (
            <button
              key={cat}
              onClick={() => setSelectedCategory(selectedCategory === cat ? null : cat)}
              className={`rounded-full px-3 py-1 text-sm font-medium transition ${
                selectedCategory === cat
                  ? 'bg-indigo-600 text-white'
                  : 'bg-gray-100 text-gray-700 hover:bg-gray-200 dark:bg-gray-700 dark:text-gray-300 dark:hover:bg-gray-600'
              }`}
            >
              {cat}
            </button>
          ))}
        </div>
      </div>

      <div className="flex flex-wrap gap-3">
        <select
          value={selectedMcVersion}
          onChange={(e) => setSelectedMcVersion(e.target.value)}
          className="rounded-lg border bg-white px-3 py-2 text-sm dark:border-gray-700 dark:bg-gray-800"
        >
          <option value="all">All MC Versions</option>
          {['1.21.11', '1.21.10', '1.21.9', '1.21.8', '1.21.7', '1.21.6', '1.21.5', '1.21.4', '1.21.3', '1.21.2', '1.21.1', '1.21', '1.20.6', '1.20.5', '1.20.4', '1.20.2', '1.20.1', '1.20', '1.19.4', '1.19.3', '1.19.2', '1.19', '1.18.2', '1.18.1', '1.18'].map((v) => (
            <option key={v} value={v}>{v}</option>
          ))}
        </select>
        <select
          value={selectedLoader}
          onChange={(e) => setSelectedLoader(e.target.value)}
          className="rounded-lg border bg-white px-3 py-2 text-sm dark:border-gray-700 dark:bg-gray-800"
        >
          <option value="all">All Loaders</option>
          <option value="fabric">Fabric</option>
          <option value="quilt">Quilt</option>
          <option value="neoforge">NeoForge</option>
          <option value="forge">Forge</option>
        </select>
        <select
          value={selectedSort}
          onChange={(e) => setSelectedSort(e.target.value as 'net_score' | 'velocity' | 'newest')}
          className="rounded-lg border bg-white px-3 py-2 text-sm dark:border-gray-700 dark:bg-gray-800"
        >
          <option value="net_score">Default</option>
          <option value="velocity">Trending</option>
          <option value="newest">Newest</option>
        </select>
      </div>

      <div>
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={`Search ${typeLabel.toLowerCase()}...`}
          className="w-full rounded-lg border bg-white px-4 py-2 dark:border-gray-700 dark:bg-gray-800"
        />
      </div>

      {filtered.length === 0 ? (
        <p className="text-gray-600 dark:text-gray-400">No results match your filters.</p>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {filtered.map((item) => (
            <Link
              key={item.id}
              href={`${typePath}/${item.id}`}
              className="flex flex-col rounded-xl border bg-white p-5 shadow-sm transition hover:shadow-md dark:border-gray-700 dark:bg-gray-800"
            >
              <div className="flex items-start gap-3">
                <ItemIcon item={item} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-start justify-between gap-2">
                    <h2 className="font-semibold truncate">{item.name}</h2>
                    <span className="shrink-0 rounded-full bg-indigo-100 px-2 py-0.5 text-xs font-medium text-indigo-700 dark:bg-indigo-900 dark:text-indigo-200">
                      {item.net_score}
                    </span>
                  </div>
                  {item.author && (
                    <p className="text-xs text-gray-500 dark:text-gray-400">by {item.author}</p>
                  )}
                </div>
              </div>
              <p className="mt-2 line-clamp-3 flex-1 text-sm text-gray-600 dark:text-gray-400">
                {item.curator_note || item.description}
              </p>
              <div className="mt-4 flex flex-wrap gap-2">
                {item.categories.slice(0, 4).map((cat) => (
                  <span
                    key={cat}
                    className="rounded-md bg-gray-100 px-2 py-0.5 text-xs text-gray-700 dark:bg-gray-700 dark:text-gray-300"
                  >
                    {cat}
                  </span>
                ))}
                {item.categories.length > 4 && (
                  <span className="rounded-md bg-gray-100 px-2 py-0.5 text-xs text-gray-500 dark:bg-gray-700 dark:text-gray-400">
                    +{item.categories.length - 4}
                  </span>
                )}
              </div>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
