import type { InstalledContentRow, UpdateInfo } from '../../lib/tauri';
import type React from 'react';

export type InstalledContentType = 'mod' | 'resourcepack' | 'shader' | 'datapack';

export type ContentColumn =
  | 'name'
  | 'author'
  | 'source'
  | 'size'
  | 'installed'
  | 'enabled'
  | 'actions'
  | 'version'
  | 'categories'
  | 'curation'
  | 'agora_score'
  | 'modrinth_downloads'
  | 'update_status'
  | 'loader_mod_id';

export type SortColumn =
  | 'name'
  | 'author'
  | 'source'
  | 'size'
  | 'installed'
  | 'enabled'
  | 'filename'
  | 'agora_score'
  | 'modrinth_downloads';

export interface SortState {
  column: SortColumn;
  direction: 'asc' | 'desc' | null;
}

/**
 * How the installed list is grouped. Every mode is *derived* from data the row
 * already carries, so grouping needs no stored state of its own.
 */
export type GroupMode = 'none' | 'pack' | 'category' | 'source';

export interface ContentGroup {
  /** Stable identity for collapse state; not shown to the user. */
  key: string;
  label: string;
  rows: InstalledContentRow[];
}

export interface ContentFilters {
  categories: string[];
  curation: string;
  source: string;
  enabled: 'all' | 'enabled' | 'disabled' | 'missing';
}

export interface InstalledContentPanelProps {
  contentType: InstalledContentType;
  rows: InstalledContentRow[];
  locked: boolean;
  onAdd: () => void;
  addLabel: string;
  onToggle: (row: InstalledContentRow) => Promise<boolean | void>;
  onBulkToggle: (rows: InstalledContentRow[], enabled: boolean) => Promise<boolean>;
  onBulkRemove: (rows: InstalledContentRow[]) => boolean;
  onRemove: (row: InstalledContentRow) => void;
  onOpenDetails?: (row: InstalledContentRow) => void;
  onRevealFile?: (row: InstalledContentRow) => void;
  onSetCustomIcon?: (row: InstalledContentRow) => void;
  onCheckUpdates?: () => Promise<UpdateInfo[]>;
  onApplyUpdate?: (row: InstalledContentRow, update: UpdateInfo) => void;
  /** Apply every available update for this panel as one reviewed transaction. */
  onUpdateAll?: (updates: UpdateInfo[]) => void;
  /** Pin or unpin a row against updates. */
  onTogglePin?: (row: InstalledContentRow, pinned: boolean) => void;
  /** Open the "why is this mod here?" trace for a row. */
  onExplainPresence?: (row: InstalledContentRow) => void;
  /**
   * Last persisted update check, read from cache so results survive navigation
   * and restart. Must be a stable reference — a fresh array each render would
   * re-seed on every parent render.
   */
  initialUpdates?: UpdateInfo[] | null;
  onError?: (message: string) => void;
  onDrop?: React.DragEventHandler<HTMLElement>;
  extraActions?: React.ReactNode;
  iconForRow?: (row: InstalledContentRow) => string | null;
}
