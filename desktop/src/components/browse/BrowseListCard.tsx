import { Check, ExternalLink } from 'lucide-react';
import { BrowseIcon } from './BrowseIcon';
import { BrowseContextLabels, BrowseStats, BrowseVersions, CuratedBadge } from './BrowseContextLabels';
import type { BrowseCardProps } from './types';
import { sourceLabel } from './sourceLabel';
import { downloadSourceLabel, downloadSourcesOf, openExternalUrl } from '../../lib/tauri';

function sourceSummary(item: BrowseCardProps['item']): string {
  const source = sourceLabel(item.source);
  if (item.author) return `by ${item.author} · ${source}`;
  if (item.registryItem?.download_strategy) {
    // Name the preferred source, and say how many fallbacks stand behind it
    // rather than listing every one in a card subtitle.
    const sources = downloadSourcesOf(item.registryItem);
    const preferred = downloadSourceLabel(sources[0].strategy);
    const fallbacks = sources.length - 1;
    return fallbacks > 0
      ? `${source} · ${preferred} +${fallbacks} fallback${fallbacks > 1 ? 's' : ''}`
      : `${source} · ${preferred}`;
  }
  return source;
}

export function BrowseListCard({ item, context, onSelectMod, selected = false, onToggleSelect }: BrowseCardProps) {
  return (
    <li
      className={[
        'browse-list-card',
        onToggleSelect ? 'browse-card--selectable' : '',
        selected ? 'browse-card--selected' : '',
      ].join(' ')}
      role={onToggleSelect ? 'button' : undefined}
      tabIndex={onToggleSelect ? 0 : undefined}
      aria-pressed={onToggleSelect ? selected : undefined}
      onClick={onToggleSelect}
      onKeyDown={(event) => {
        if (!onToggleSelect || event.target !== event.currentTarget) return;
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          onToggleSelect();
        }
      }}
    >
      {selected && (
        <span className="browse-card-check" aria-hidden>
          <Check size={14} />
        </span>
      )}
      <div className="browse-list-card__layout">
        <BrowseIcon iconUrl={item.iconUrl} name={item.name} className="browse-list-card__icon" />
        <div className="browse-list-card__main">
          <div className="browse-card-title-row">
            <h3 className="browse-card-title">{item.name}</h3>
            {item.source === 'curated' && <CuratedBadge />}
          </div>
          <p className="browse-card-source">{sourceSummary(item)}</p>
          {item.description && <p className="browse-list-card__description">{item.description}</p>}
          {item.categories.length > 0 && (
            <div className="browse-list-card__categories browse-category-list">
              {item.categories.slice(0, 4).map((category) => <span key={category}>{category.replace(/[-_]/g, ' ')}</span>)}
            </div>
          )}
          <div className="browse-list-card__inline-stats"><BrowseStats item={item} /></div>
          <BrowseVersions item={item} />
          {context?.whyRecommended && <p className="browse-recommendation">Why: {context.whyRecommended}</p>}
        </div>
        <aside className="browse-list-card__side">
          <BrowseStats item={item} />
          <div className="browse-list-card__actions">
            <BrowseContextLabels context={context} />
            <div className="browse-card-action-row">
              <button
                type="button"
                onClick={(event) => {
                  event.stopPropagation();
                  onSelectMod?.(item.id);
                }}
                className="browse-primary-action"
              >
                View Details
              </button>
              {item.sourcePageUrl && (
                <button
                  type="button"
                  onClick={(event) => {
                    // `target="_blank"` is inert inside the Tauri webview, so
                    // the link has to go through the OS opener instead.
                    event.stopPropagation();
                    void openExternalUrl(item.sourcePageUrl!);
                  }}
                  className="browse-source-link"
                >
                  View source <ExternalLink aria-hidden size={12} />
                </button>
              )}
            </div>
          </div>
        </aside>
      </div>
    </li>
  );
}
