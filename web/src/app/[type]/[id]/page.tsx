import Link from 'next/link';
import { notFound } from 'next/navigation';
import MarkdownRenderer from '@/components/MarkdownRenderer';
import Reviews from '@/components/Reviews';
import {
  getItemById,
  getItemIds,
  contentTypeFromPath,
  contentTypeLabel,
  contentTypePath,
  CONTENT_TYPES,
  type RegistryItem,
} from '@/lib/db';

interface DetailPageProps {
  params: Promise<{ type: string; id: string }>;
}

export async function generateStaticParams() {
  const params: { type: string; id: string }[] = [];
  for (const type of CONTENT_TYPES) {
    const ids = await getItemIds(type);
    params.push(...ids.map((id) => ({ type: `${type}s`, id })));
  }
  return params;
}

function StatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    active: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200',
    under_review: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-200',
    archived: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200',
    deprecated: 'bg-gray-100 text-gray-800 dark:bg-gray-700 dark:text-gray-200',
  };
  const labels: Record<string, string> = {
    active: 'Active',
    under_review: 'Under Review',
    archived: 'Archived',
    deprecated: 'Deprecated',
  };
  return (
    <span className={`rounded-full px-3 py-1 text-xs font-medium ${colors[status] ?? colors.active}`}>
      {labels[status] ?? status}
    </span>
  );
}

function SourceLinks({ item }: { item: RegistryItem }) {
  const links: { label: string; url: string }[] = [];

  if (item.github_repository_url) {
    links.push({ label: 'GitHub Repository', url: item.github_repository_url });
  }
  if (item.github_releases_url) {
    links.push({ label: 'GitHub Releases', url: item.github_releases_url });
  }
  if (item.github_issues_url) {
    links.push({ label: 'Issue Tracker', url: item.github_issues_url });
  }
  if (item.modrinth_url) {
    links.push({ label: 'Modrinth Page', url: item.modrinth_url });
  }
  if (item.page_url && item.page_url !== item.modrinth_url && item.page_url !== item.github_repository_url) {
    links.push({ label: 'Project Website', url: item.page_url });
  }

  if (links.length === 0) return null;

  return (
    <div className="panel">
      <h2 className="rule-gold mb-4 text-lg font-semibold text-ink">Source Links</h2>
      <div className="flex flex-wrap gap-2">
        {links.map((link) => (
          <a
            key={link.label}
            href={link.url}
            target="_blank"
            rel="noopener noreferrer"
            className="btn-ghost px-4 py-2 text-sm"
          >
            {link.label}
          </a>
        ))}
      </div>
    </div>
  );
}

export default async function DetailPage({ params }: DetailPageProps) {
  const { type, id } = await params;
  const contentType = contentTypeFromPath(type);
  if (!contentType) {
    notFound();
  }

  const item = await getItemById(id);
  if (!item) {
    notFound();
  }

  return (
    <div className="space-y-8">
      {item.is_immune && (
        <div
          className="rounded-xl border border-blue-300 bg-blue-50 px-4 py-3 text-sm text-blue-900 dark:border-blue-700 dark:bg-blue-950 dark:text-blue-200"
          role="status"
        >
          <div className="flex items-center gap-2 font-semibold">
            <span aria-hidden="true">🛡️</span>
            <span>Curator Shield Active</span>
          </div>
          <p className="mt-1 text-xs opacity-90">
            This entry is immune to community demotion and curated by Agora maintainers.
          </p>
        </div>
      )}
      <div className="panel">
        <Link
          href={contentTypePath(contentType)}
          className="text-sm text-indigo-600 hover:underline dark:text-indigo-400"
        >
          ← Back to {contentTypeLabel(contentType)}
        </Link>
        <div className="mt-2 flex flex-wrap items-center gap-3">
          <h1 className="text-3xl font-bold">{item.name}</h1>
          <StatusBadge status={item.status} />
        </div>
        {item.author && (
          <p className="text-gray-600 dark:text-gray-400">by {item.author}</p>
        )}
        <p className="text-gray-600 dark:text-gray-400">
          {contentTypeLabel(contentType)} · {item.download_strategy}
          {item.license && <> · {item.license}</>}
        </p>
      </div>

      {item.icon_url && (() => {
        let url: URL | null = null;
        try { url = new URL(item.icon_url); } catch { /* invalid */ }
        const safe = url && (url.protocol === 'https:');
        if (!safe) return null;
        return (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={item.icon_url!}
            alt={`${item.name} icon`}
            className="h-24 w-24 rounded-xl border object-contain dark:border-gray-700"
          />
        );
      })()}

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <div className="panel-inset p-4 text-center">
          <div className="text-2xl font-bold text-green-700 dark:text-green-400">{item.net_score}</div>
          <div className="text-xs text-gray-500 dark:text-gray-400">Net Score</div>
        </div>
        <div className="panel-inset p-4 text-center">
          <div className="text-2xl font-bold text-blue-700 dark:text-blue-400">▲ {item.upvotes}</div>
          <div className="text-xs text-gray-500 dark:text-gray-400">Upvotes</div>
        </div>
        <div className="panel-inset p-4 text-center">
          <div className="text-2xl font-bold text-red-700 dark:text-red-400">▼ {item.downvotes}</div>
          <div className="text-xs text-gray-500 dark:text-gray-400">Downvotes</div>
        </div>
        <div className="panel-inset p-4 text-center">
          <div className="text-2xl font-bold">{item.velocity.toFixed(1)}</div>
          <div className="text-xs text-gray-500 dark:text-gray-400">Velocity</div>
        </div>
      </div>

      <div className="panel">
        <h2 className="rule-gold mb-4 text-xl font-semibold text-ink">Curator Note</h2>
        {item.curator_note ? (
          <MarkdownRenderer content={item.curator_note} />
        ) : (
          <p className="text-sm text-gray-500 italic">No curator note yet.</p>
        )}
      </div>

      {item.description && item.description !== item.curator_note && (
        <div className="panel">
          <h2 className="rule-gold mb-4 text-xl font-semibold text-ink">Upstream Description</h2>
          <MarkdownRenderer content={item.description} />
        </div>
      )}

      <SourceLinks item={item} />

      {item.categories.length > 0 && (
        <div className="panel">
          <h2 className="rule-gold mb-4 text-lg font-semibold text-ink">Categories</h2>
          <div className="flex flex-wrap gap-2">
            {item.categories.map((cat) => (
              <span
                key={cat}
                className="rounded-md border border-gold/35 bg-gold/12 px-3 py-1 text-sm font-semibold text-gold-bright"
              >
                {cat}
              </span>
            ))}
            {item.community_categories.map((cat) => (
              <span
                key={cat}
                className="rounded-md border border-gold/15 bg-canvas/70 px-3 py-1 text-sm text-ink-muted"
              >
                {cat}
              </span>
            ))}
          </div>
        </div>
      )}

      {item.compatible_versions.length > 0 && (
        <div className="panel">
          <h2 className="rule-gold mb-4 text-lg font-semibold text-ink">Compatible Versions</h2>
          <ul className="list-disc space-y-1 pl-6">
            {item.compatible_versions.map((v, i) => (
              <li key={i} className="text-gray-700 dark:text-gray-300">
                {v.mc_version} · {v.loader} · {v.mod_version}
              </li>
            ))}
          </ul>
        </div>
      )}

      {item.body_markdown && (
        <details className="panel-inset">
          <summary className="cursor-pointer px-6 py-4 text-lg font-semibold">
            Full Description
          </summary>
          <div className="px-6 pb-6">
            <MarkdownRenderer content={item.body_markdown} />
          </div>
        </details>
      )}

      <div className="panel-inset p-4">
        <div className="grid gap-2 text-xs sm:grid-cols-2 lg:grid-cols-3">
          <div>
            <span className="font-semibold text-gray-500">Source:</span>{' '}
            <code className="rounded bg-gray-100 px-1 dark:bg-gray-700">{item.source_identifier}</code>
          </div>
          <div>
            <span className="font-semibold text-gray-500">Strategy:</span>{' '}
            <span>{item.download_strategy}</span>
          </div>
          <div>
            <span className="font-semibold text-gray-500">SHA-256:</span>{' '}
            <code className="rounded bg-gray-100 px-1 text-xs dark:bg-gray-700">{item.sha256.slice(0, 16)}...</code>
          </div>
          {item.date_added && (
            <div>
              <span className="font-semibold text-gray-500">Added:</span>{' '}
              <span>{item.date_added}</span>
            </div>
          )}
          {item.source_updated_at && (
            <div>
              <span className="font-semibold text-gray-500">Updated:</span>{' '}
              <span>{item.source_updated_at}</span>
            </div>
          )}
        </div>
      </div>

      <Reviews itemId={item.id} />
    </div>
  );
}
