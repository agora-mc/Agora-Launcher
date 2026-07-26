import type { InstalledContentRow } from '../../lib/tauri';
import type { ContentColumn, ContentFilters, SortColumn, SortState } from './types';

export function normalizeSearchText(value: string): string {
  return value.trim().replace(/\s+/g, ' ').toLocaleLowerCase();
}

export function searchInstalledContent(rows: InstalledContentRow[], query: string): InstalledContentRow[] {
  const normalized = normalizeSearchText(query);
  if (!normalized) return rows;
  return rows.filter((row) => normalizeSearchText([
    row.display_name,
    row.filename,
    row.author ?? '',
    row.version ?? '',
    row.loader_mod_id ?? '',
    row.registry_id ?? '',
    row.modrinth_id ?? '',
    row.categories.join(' '),
    row.source_label,
  ].join(' ')).includes(normalized));
}

export function filterInstalledContent(rows: InstalledContentRow[], filters: ContentFilters): InstalledContentRow[] {
  return rows.filter((row) => {
    if (filters.categories.length > 0 && !filters.categories.some((category) => row.categories.includes(category))) return false;
    if (filters.curation !== 'all' && row.curation_status !== filters.curation) return false;
    if (filters.source !== 'all' && row.source_label !== filters.source) return false;
    if (filters.enabled === 'enabled' && !row.enabled) return false;
    if (filters.enabled === 'disabled' && row.enabled) return false;
    if (filters.enabled === 'missing' && row.file_present) return false;
    return true;
  });
}

function nullableCompare(a: string | number | null | undefined, b: string | number | null | undefined): number {
  const aUnknown = a == null || a === '';
  const bUnknown = b == null || b === '';
  if (aUnknown || bUnknown) {
    if (aUnknown && bUnknown) return 0;
    return aUnknown ? 1 : -1;
  }
  if (typeof a === 'number' && typeof b === 'number') return a - b;
  return String(a).localeCompare(String(b), undefined, { sensitivity: 'base', numeric: true });
}

function sortValue(row: InstalledContentRow, column: SortColumn): string | number | null {
  switch (column) {
    case 'name': return row.display_name;
    case 'filename': return row.filename;
    case 'author': return row.author;
    case 'source': return row.source_label;
    case 'size': return row.size_bytes;
    case 'installed': {
      const timestamp = Date.parse(row.installed_at);
      return Number.isNaN(timestamp) ? null : timestamp;
    }
    case 'enabled': return row.enabled ? 1 : 0;
    case 'agora_score': return row.agora_score;
    case 'modrinth_downloads': return row.modrinth_downloads;
  }
}

function unknownSortValue(row: InstalledContentRow, column: SortColumn): boolean {
  const value = sortValue(row, column);
  return value == null || value === '';
}

export function compareInstalledContent(a: InstalledContentRow, b: InstalledContentRow, column: SortColumn): number {
  const primary = nullableCompare(sortValue(a, column), sortValue(b, column));
  if (primary !== 0) return primary;
  const name = nullableCompare(a.display_name, b.display_name);
  if (name !== 0) return name;
  const filename = nullableCompare(a.filename, b.filename);
  return filename !== 0 ? filename : a.key.localeCompare(b.key);
}

export function sortInstalledContent(rows: InstalledContentRow[], sort: SortState): InstalledContentRow[] {
  if (!sort.direction) return [...rows];
  const direction = sort.direction === 'asc' ? 1 : -1;
  return [...rows].sort((a, b) => {
    const aUnknown = unknownSortValue(a, sort.column);
    const bUnknown = unknownSortValue(b, sort.column);
    if (aUnknown || bUnknown) {
      if (aUnknown && bUnknown) return 0;
      return aUnknown ? 1 : -1;
    }
    return compareInstalledContent(a, b, sort.column) * direction;
  });
}

export function deriveAvailableFilters(rows: InstalledContentRow[]) {
  return {
    categories: [...new Set(rows.flatMap((row) => row.categories))].sort((a, b) => a.localeCompare(b)),
    sources: [...new Set(rows.map((row) => row.source_label))].sort((a, b) => a.localeCompare(b)),
  };
}

export function formatBytes(value: number | null): string {
  if (value == null) return 'Missing';
  if (value < 1024) return `${value} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let amount = value;
  let unit = -1;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit += 1;
  }
  return `${amount >= 10 ? amount.toFixed(0) : amount.toFixed(1)} ${units[unit]}`;
}

export function formatCompactNumber(value: number | null): string {
  if (value == null) return 'Unavailable';
  return new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 }).format(value);
}

export function formatInstalledDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return 'Unknown';
  return new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', year: 'numeric' }).format(date);
}

export const defaultColumns: ContentColumn[] = ['name', 'author', 'source', 'size', 'installed', 'enabled', 'update_status', 'actions'];
