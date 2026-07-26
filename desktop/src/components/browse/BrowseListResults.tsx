import { BrowseListCard } from './BrowseListCard';
import type { BrowseItem, BrowseItemContext } from './types';

export function BrowseListResults({ items, contextFor, onSelectMod }: {
  items: BrowseItem[];
  contextFor: (item: BrowseItem) => BrowseItemContext | null;
  onSelectMod?: (id: string) => void;
}) {
  return (
    <div className="browse-list-results-shell">
      <ul className="browse-list-results" data-testid="browse-list-results">
        {items.map((item) => (
          <BrowseListCard key={`${item.source}:${item.id}`} item={item} context={contextFor(item)} onSelectMod={onSelectMod} />
        ))}
      </ul>
    </div>
  );
}
