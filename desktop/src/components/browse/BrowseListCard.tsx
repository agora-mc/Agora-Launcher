import { ExternalLink } from 'lucide-react';
import { BrowseIcon } from './BrowseIcon';
import { BrowseContextLabels, BrowseStats, BrowseVersions, CuratedBadge } from './BrowseContextLabels';
import type { BrowseCardProps } from './types';

function sourceSummary(item: BrowseCardProps['item']): string {
  const source = item.source === 'curated' ? 'Agora Registry' : 'Modrinth';
  if (item.author) return `by ${item.author} · ${source}`;
  if (item.registryItem?.download_strategy) {
    return `${source} · ${item.registryItem.download_strategy.replace(/_/g, ' ').replace(/\b\w/g, (letter) => letter.toUpperCase())}`;
  }
  return source;
}

export function BrowseListCard({ item, context, onSelectMod }: BrowseCardProps) {
  return (
    <li className="browse-list-card">
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
          <BrowseContextLabels context={context} />
          <BrowseStats item={item} />
          <button type="button" onClick={() => onSelectMod?.(item.id)} className="browse-primary-action">
            View Details
          </button>
          {item.sourcePageUrl && (
            <a href={item.sourcePageUrl} target="_blank" rel="noopener noreferrer" className="browse-source-link">
              View source <ExternalLink aria-hidden size={12} />
              </a>
          )}
        </aside>
      </div>
    </li>
  );
}
