import type { InstalledContentRow } from '../../lib/tauri';
import type { ContentColumn, ContentFilters, ContentGroup, GroupMode, SortColumn, SortState } from './types';

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
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
}

export const defaultColumns: ContentColumn[] = ['name', 'author', 'source', 'size', 'installed', 'enabled', 'update_status', 'actions'];

/**
 * Split rows into display groups. Purely derived — every mode reads a field the
 * row already carries, so nothing is persisted and a row never appears twice.
 *
 * Sort order within a group is whatever the caller already applied; only the
 * *group* order is decided here.
 */
export function groupInstalledContent(rows: InstalledContentRow[], mode: GroupMode): ContentGroup[] {
  if (mode === 'none') return [{ key: 'all', label: '', rows }];

  const buckets = new Map<string, ContentGroup>();
  const push = (key: string, label: string, row: InstalledContentRow) => {
    const existing = buckets.get(key);
    if (existing) existing.rows.push(row);
    else buckets.set(key, { key, label, rows: [row] });
  };

  for (const row of rows) {
    if (mode === 'pack') {
      // Two buckets only, so an instance with no pack still reads sensibly.
      if (row.pack_managed) push('pack', 'From the modpack', row);
      else push('user', 'Added by you', row);
    } else if (mode === 'source') {
      push(`source:${row.source_label}`, row.source_label, row);
    } else {
      // A row can carry several categories; grouping on the first keeps every
      // row in exactly one bucket. Duplicating rows across groups would break
      // select-all and the counts.
      const category = row.categories[0];
      if (category) push(`category:${category}`, category, row);
      else push('category:__none__', 'Uncategorized', row);
    }
  }

  const groups = [...buckets.values()];
  if (mode === 'pack') {
    // Pack content first: it is the part the user did not choose and most
    // wants to see distinguished.
    return groups.sort((a, b) => (a.key === 'pack' ? -1 : b.key === 'pack' ? 1 : 0));
  }
  // "Uncategorized" sinks to the bottom; everything else is alphabetical.
  return groups.sort((a, b) => {
    if (a.key === 'category:__none__') return 1;
    if (b.key === 'category:__none__') return -1;
    return a.label.localeCompare(b.label);
  });
}
