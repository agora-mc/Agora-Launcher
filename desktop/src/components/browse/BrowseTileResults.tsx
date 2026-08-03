import { BrowseTileCard } from './BrowseTileCard';
import type { BrowseItem, BrowseItemContext } from './types';

export function BrowseTileResults({ items, contextFor, onSelectMod, selectedItems, onToggleSelect }: {
  items: BrowseItem[];
  contextFor: (item: BrowseItem) => BrowseItemContext | null;
  onSelectMod?: (id: string) => void;
  selectedItems?: ReadonlyMap<string, BrowseItem>;
  onToggleSelect?: (item: BrowseItem) => void;
}) {
  return (
    <div className="browse-tile-results" data-testid="browse-tile-results">
      {items.map((item) => {
        const key = `${item.source}:${item.id}`;
        return (
          <BrowseTileCard
            key={key}
            item={item}
            context={contextFor(item)}
            onSelectMod={onSelectMod}
            selected={selectedItems?.has(key) ?? false}
            onToggleSelect={onToggleSelect ? () => onToggleSelect(item) : undefined}
          />
        );
      })}
    </div>
  );
}
