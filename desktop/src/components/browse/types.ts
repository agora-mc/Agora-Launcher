import type { BrowseItemCached } from '../../lib/tauri';

export type BrowseItem = BrowseItemCached;

export interface BrowseItemContext {
  instanceName: string;
  minecraftVersion: string;
  loader: string;
  compatibility: 'compatible' | 'major_match' | '';
  installed: boolean;
  updateAvailable: boolean;
  whyRecommended: string | null;
}

export interface BrowseCardProps {
  item: BrowseItem;
  context: BrowseItemContext | null;
  onSelectMod?: (id: string) => void;
}
