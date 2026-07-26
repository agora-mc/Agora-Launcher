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
  onError?: (message: string) => void;
  onDrop?: React.DragEventHandler<HTMLElement>;
  extraActions?: React.ReactNode;
  iconForRow?: (row: InstalledContentRow) => string | null;
}
