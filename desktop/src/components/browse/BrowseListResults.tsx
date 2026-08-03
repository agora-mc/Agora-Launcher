import { BrowseListCard } from './BrowseListCard';
import type { BrowseItem, BrowseItemContext } from './types';

export function BrowseListResults({ items, contextFor, onSelectMod, selectedItems, onToggleSelect }: {
  items: BrowseItem[];
  contextFor: (item: BrowseItem) => BrowseItemContext | null;
  onSelectMod?: (id: string) => void;
  selectedItems?: ReadonlyMap<string, BrowseItem>;
  onToggleSelect?: (item: BrowseItem) => void;
}) {
  return (
    <div className="browse-list-results-shell">
      <ul className="browse-list-results" data-testid="browse-list-results">
        {items.map((item) => {
          const key = `${item.source}:${item.id}`;
          return (
            <BrowseListCard
              key={key}
              item={item}
              context={contextFor(item)}
              onSelectMod={onSelectMod}
              selected={selectedItems?.has(key) ?? false}
              onToggleSelect={onToggleSelect ? () => onToggleSelect(item) : undefined}
            />
          );
        })}
      </ul>
    </div>
  );
}
