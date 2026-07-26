import { BrowseTileCard } from './BrowseTileCard';
import type { BrowseItem, BrowseItemContext } from './types';

export function BrowseTileResults({ items, contextFor, onSelectMod }: {
  items: BrowseItem[];
  contextFor: (item: BrowseItem) => BrowseItemContext | null;
  onSelectMod?: (id: string) => void;
}) {
  return (
    <div className="browse-tile-results" data-testid="browse-tile-results">
      {items.map((item) => (
        <BrowseTileCard key={`${item.source}:${item.id}`} item={item} context={contextFor(item)} onSelectMod={onSelectMod} />
      ))}
    </div>
  );
}
