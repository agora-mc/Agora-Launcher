import type { BrowseItemCached } from '../../lib/tauri';

export type BrowseItem = BrowseItemCached;

export interface BrowseItemContext {
  installed: boolean;
  whyRecommended: string | null;
}

export interface BrowseCardProps {
  item: BrowseItem;
  context: BrowseItemContext | null;
  onSelectMod?: (id: string) => void;
}
