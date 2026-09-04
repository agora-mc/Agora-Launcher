import { Check, ExternalLink } from 'lucide-react';
import { BrowseHeroMedia } from './BrowseHeroMedia';
import { BrowseContextLabels, BrowseStats, BrowseVersions, CuratedBadge } from './BrowseContextLabels';
import type { BrowseCardProps } from './types';
import { sourceLabel } from './sourceLabel';
import { openExternalUrl } from '../../lib/tauri';

export function BrowseTileCard({ item, context, onSelectMod, selected = false, onToggleSelect }: BrowseCardProps) {
  return (
    <article
      className={[
        'browse-tile-card',
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
      <BrowseHeroMedia item={item} />
      <div className="browse-tile-card__body">
        <div className="browse-card-title-row">
          <h3 className="browse-card-title">{item.name}</h3>
          {item.source === 'curated' && <CuratedBadge />}
        </div>
        <p className="browse-card-source">
          {item.author ? `by ${item.author} · ` : ''}{sourceLabel(item.source)}
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
      </div>
    </article>
  );
}
