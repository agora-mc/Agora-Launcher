import { Check, ExternalLink } from 'lucide-react';
import { BrowseHeroMedia } from './BrowseHeroMedia';
import { BrowseContextLabels, BrowseStats, BrowseVersions, CuratedBadge } from './BrowseContextLabels';
import type { BrowseCardProps } from './types';

export function BrowseTileCard({ item, context, onSelectMod, selected = false, onToggleSelect }: BrowseCardProps) {
  return (
    <article
      className={[
        'browse-tile-card',
        onToggleSelect ? 'browse-card--selectable' : '',
        selected ? 'browse-card--selected' : '',
      ].join(' ')}
      onClick={onToggleSelect}
    >
      {selected && (
        <span className="browse-card-check" aria-hidden>
          <Check size={14} />
        </span>
      )}
      <BrowseHeroMedia item={item} />
      <div className="browse-tile-card__body">
        <div className="browse-card-title-row">
          <h3 className="browse-card-title">{item.name}</h3>
          {item.source === 'curated' && <CuratedBadge />}
        </div>
        <p className="browse-card-source">
          {item.author ? `by ${item.author} · ` : ''}{item.source === 'curated' ? 'Agora Registry' : 'Modrinth'}
        </p>
        {item.description ? (
          <p className="browse-tile-card__description">{item.description}</p>
        ) : (
          <p className="browse-tile-card__description browse-tile-card__description--empty">No description available.</p>
        )}
        <BrowseStats item={item} />
        <BrowseVersions item={item} />
        {item.categories.length > 0 && (
          <div className="browse-category-list">
            {item.categories.slice(0, 3).map((category) => <span key={category}>{category.replace(/[-_]/g, ' ')}</span>)}
          </div>
        )}
        {context?.whyRecommended && <p className="browse-recommendation">Why: {context.whyRecommended}</p>}
        <div className="browse-tile-card__actions">
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
              <a
                href={item.sourcePageUrl}
                target="_blank"
                rel="noopener noreferrer"
                onClick={(event) => event.stopPropagation()}
                className="browse-source-link"
              >
                View source <ExternalLink aria-hidden size={12} />
              </a>
            )}
          </div>
        </div>
      </div>
    </article>
  );
}
