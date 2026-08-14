import { useEffect, useMemo, useState } from 'react';
import { ArrowUpCircle, ChevronDown, ChevronUp, ChevronsUpDown, MoreHorizontal, Search, Trash2 } from 'lucide-react';
import { Switch } from '../ui/switch';
import { formatError } from '../../lib/tauri';
import type { InstalledContentRow, UpdateInfo } from '../../lib/tauri';
import type { ContentColumn, ContentFilters, InstalledContentPanelProps, SortColumn, SortState } from './types';
import {
  defaultColumns,
  deriveAvailableFilters,
  filterInstalledContent,
  formatBytes,
  formatCompactNumber,
  formatInstalledDate,
  normalizeSearchText,
  searchInstalledContent,
  sortInstalledContent,
} from './contentTableState';

const columnLabels: Record<ContentColumn, string> = {
  name: 'Name / Filename', author: 'Author', source: 'Source', size: 'Size', installed: 'Installed',
  enabled: 'Enabled', actions: 'Actions', version: 'Version', categories: 'Categories', curation: 'Curation',
  agora_score: 'Agora score', modrinth_downloads: 'Modrinth downloads', update_status: 'Update status', loader_mod_id: 'Loader mod ID',
};

const allColumns: ContentColumn[] = ['name', 'author', 'source', 'size', 'installed', 'enabled', 'update_status', 'actions', 'version', 'categories', 'curation', 'agora_score', 'modrinth_downloads', 'loader_mod_id'];
const titleForType: Record<InstalledContentPanelProps['contentType'], string> = {
  mod: 'Installed Mods', resourcepack: 'Installed Resource Packs', shader: 'Installed Shaders', datapack: 'Installed Data Packs',
};

function preferenceKey(contentType: InstalledContentPanelProps['contentType']) {
  return `agora.installed-content.v2.${contentType}`;
}

function loadPreferences(contentType: InstalledContentPanelProps['contentType']): { columns: ContentColumn[]; sort: SortState } {
  try {
    const stored = JSON.parse(localStorage.getItem(preferenceKey(contentType)) ?? '{}') as { columns?: ContentColumn[]; sort?: SortState };
    const columns = stored.columns?.filter((column) => columnLabels[column]) ?? defaultColumns;
    const sort = stored.sort?.column && stored.sort.direction !== undefined
      ? stored.sort
      : { column: 'name' as const, direction: 'asc' as const };
    return { columns: columns.length > 0 ? columns : defaultColumns, sort };
  } catch {
    return { columns: defaultColumns, sort: { column: 'name', direction: 'asc' } };
  }
}

function savePreferences(contentType: InstalledContentPanelProps['contentType'], columns: ContentColumn[], sort: SortState) {
  try {
    localStorage.setItem(preferenceKey(contentType), JSON.stringify({ version: 2, columns, sort }));
  } catch {
    // Preferences are best effort in private browsing and restricted webviews.
  }
}

function curationLabel(value: string) {
  return value === 'under_review' ? 'Under review' : value.charAt(0).toUpperCase() + value.slice(1);
}

function SortIndicator({ column, sort }: { column: SortColumn; sort: SortState }) {
  if (sort.column !== column || !sort.direction) return <ChevronsUpDown className="h-3.5 w-3.5 text-muted-foreground" aria-hidden="true" />;
  return sort.direction === 'asc'
    ? <ChevronUp className="h-3.5 w-3.5" aria-hidden="true" />
    : <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />;
}

export function InstalledContentPanel(props: InstalledContentPanelProps) {
  const preferences = useMemo(() => loadPreferences(props.contentType), [props.contentType]);
  const [query, setQuery] = useState('');
  const [filters, setFilters] = useState<ContentFilters>({ categories: [], curation: 'all', source: 'all', enabled: 'all' });
  const [columns, setColumns] = useState<ContentColumn[]>(preferences.columns);
  const [sort, setSort] = useState<SortState>(preferences.sort);
  const [pending, setPending] = useState<Record<string, boolean>>({});
  const [optimisticEnabled, setOptimisticEnabled] = useState<Record<string, boolean>>({});
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set());
  const [bulkBusy, setBulkBusy] = useState(false);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [updatesByFilename, setUpdatesByFilename] = useState<Record<string, UpdateInfo>>({});
  const [updatesChecked, setUpdatesChecked] = useState(false);

  useEffect(() => {
    setColumns(preferences.columns);
    setSort(preferences.sort);
    setQuery('');
    setFilters({ categories: [], curation: 'all', source: 'all', enabled: 'all' });
    setSelectedKeys(new Set());
    setUpdatesByFilename({});
    setUpdatesChecked(false);
  }, [preferences]);

  useEffect(() => {
    savePreferences(props.contentType, columns, sort);
  }, [columns, sort, props.contentType]);

  useEffect(() => {
    setOptimisticEnabled({});
    setSelectedKeys(new Set());
  }, [props.rows]);

  const rows = useMemo(() => props.rows.map((row) => optimisticEnabled[row.key] === undefined ? row : { ...row, enabled: optimisticEnabled[row.key] }), [props.rows, optimisticEnabled]);
  const available = useMemo(() => deriveAvailableFilters(rows), [rows]);
  const visibleRows = useMemo(() => sortInstalledContent(
    filterInstalledContent(searchInstalledContent(rows, query), filters),
    sort,
  ), [rows, query, filters, sort]);
  const filterCount = filters.categories.length + (filters.curation === 'all' ? 0 : 1) + (filters.source === 'all' ? 0 : 1) + (filters.enabled === 'all' ? 0 : 1);
  const contentLabel = titleForType[props.contentType].replace('Installed ', '');
  const selectedRows = rows.filter((row) => selectedKeys.has(row.key));
  const visibleSelectedCount = visibleRows.filter((row) => selectedKeys.has(row.key)).length;
  const allVisibleSelected = visibleRows.length > 0 && visibleSelectedCount === visibleRows.length;

  const cycleSort = (column: SortColumn) => {
    if (sort.column !== column || sort.direction === null) setSort({ column, direction: 'asc' });
    else if (sort.direction === 'asc') setSort({ column, direction: 'desc' });
    else setSort({ column, direction: null });
  };

  const handleToggle = async (row: InstalledContentRow) => {
    if (props.locked || pending[row.key]) return;
    const next = !row.enabled;
    setPending((current) => ({ ...current, [row.key]: true }));
    setOptimisticEnabled((current) => ({ ...current, [row.key]: next }));
    try {
      const completed = await props.onToggle(row);
      if (completed === false) {
        setOptimisticEnabled((current) => {
          const copy = { ...current };
          delete copy[row.key];
          return copy;
        });
      }
    } catch (error) {
      setOptimisticEnabled((current) => {
        const copy = { ...current };
        delete copy[row.key];
        return copy;
      });
      props.onError?.(formatError(error));
    } finally {
      setPending((current) => {
        const copy = { ...current };
        delete copy[row.key];
        return copy;
      });
    }
  };

  const handleBulkToggle = async (enabled: boolean) => {
    const targets = selectedRows.filter((row) => row.enabled !== enabled);
    if (targets.length === 0 || props.locked || bulkBusy) return;
    setBulkBusy(true);
    setOptimisticEnabled((current) => ({
      ...current,
      ...Object.fromEntries(targets.map((row) => [row.key, enabled])),
    }));
    try {
      const completed = await props.onBulkToggle(targets, enabled);
      if (!completed) {
        setOptimisticEnabled((current) => {
          const copy = { ...current };
          targets.forEach((row) => delete copy[row.key]);
          return copy;
        });
      } else {
        setSelectedKeys(new Set());
      }
    } catch (error) {
      setOptimisticEnabled((current) => {
        const copy = { ...current };
        targets.forEach((row) => delete copy[row.key]);
        return copy;
      });
      props.onError?.(formatError(error));
    } finally {
      setBulkBusy(false);
    }
  };

  const handleBulkRemove = () => {
    if (selectedRows.length === 0 || props.locked || bulkBusy) return;
    if (props.onBulkRemove(selectedRows)) setSelectedKeys(new Set());
  };

  const toggleSelected = (key: string) => {
    setSelectedKeys((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key); else next.add(key);
      return next;
    });
  };

  const toggleVisibleSelection = () => {
    setSelectedKeys((current) => {
      const next = new Set(current);
      if (allVisibleSelected) visibleRows.forEach((row) => next.delete(row.key));
      else visibleRows.forEach((row) => next.add(row.key));
      return next;
    });
  };

  const handleCheckUpdates = async () => {
    if (!props.onCheckUpdates || checkingUpdates) return;
    setCheckingUpdates(true);
    try {
      const updates = await props.onCheckUpdates();
      setUpdatesByFilename(Object.fromEntries(updates.map((update) => [update.filename, update])));
      setUpdatesChecked(true);
    } catch (error) {
      props.onError?.(formatError(error));
    } finally {
      setCheckingUpdates(false);
    }
  };

  const updateForRow = (row: InstalledContentRow) => updatesByFilename[row.filename];
  const updateStatusForRow = (row: InstalledContentRow) => {
    if (updateForRow(row)) return 'available';
    if (!updatesChecked) return 'unchecked';
    return row.registry_id || row.modrinth_id ? 'current' : 'unavailable';
  };

  const clearFilters = () => setFilters({ categories: [], curation: 'all', source: 'all', enabled: 'all' });
  const toggleCategory = (category: string) => setFilters((current) => ({
    ...current,
    categories: current.categories.includes(category) ? current.categories.filter((value) => value !== category) : [...current.categories, category],
  }));
  const columnVisible = (column: ContentColumn) => columns.includes(column);
  const toggleColumn = (column: ContentColumn) => setColumns((current) => current.includes(column) ? current.filter((value) => value !== column) : [...current, column]);
  const onHeaderKeyDown = (event: React.KeyboardEvent, column: SortColumn) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      cycleSort(column);
    }
  };

  const renderHeader = (column: ContentColumn) => {
    const sortColumn = column === 'name' ? 'name' : column === 'size' ? 'size' : column === 'installed' ? 'installed' : column === 'author' ? 'author' : column === 'source' ? 'source' : column === 'enabled' ? 'enabled' : column === 'agora_score' ? 'agora_score' : column === 'modrinth_downloads' ? 'modrinth_downloads' : null;
    const active = sortColumn && sort.column === sortColumn && sort.direction;
    return (
      <th key={column} scope="col" aria-sort={sortColumn ? (active === 'asc' ? 'ascending' : active === 'desc' ? 'descending' : 'none') : 'none'} className="sticky top-0 z-10 bg-card px-3 py-2 text-left text-xs font-semibold text-muted-foreground whitespace-nowrap">
        {sortColumn ? (
          <button
            type="button"
            className="inline-flex items-center gap-1 text-left hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            onClick={() => cycleSort(sortColumn)}
            onKeyDown={(event) => onHeaderKeyDown(event, sortColumn)}
          >
            {columnLabels[column]} <SortIndicator column={sortColumn} sort={sort} />
          </button>
        ) : columnLabels[column]}
      </th>
    );
  };

  const renderCell = (row: InstalledContentRow, column: ContentColumn, rowIndex: number) => {
    switch (column) {
      case 'name': {
        const icon = props.iconForRow?.(row) ?? row.icon_url;
        const detailAvailable = Boolean(row.registry_id || row.modrinth_id || row.mod_jar_id);
        return <td key={column} className="min-w-[280px] max-w-[520px] px-3 py-2">
          <div className="flex items-start gap-2">
            {icon ? <img src={icon} alt="" className="h-8 w-8 shrink-0 rounded border border-border object-cover" /> : null}
            <button type="button" disabled={!detailAvailable} onClick={() => props.onOpenDetails?.(row)} className="min-w-0 text-left disabled:cursor-default enabled:cursor-pointer">
              <span className={`block truncate font-medium ${detailAvailable ? 'hover:text-primary' : ''}`}>{row.display_name}</span>
              <span className="block truncate text-xs text-muted-foreground">{row.filename}</span>
              <span className="mt-1 flex flex-wrap items-center gap-1 text-[10px] text-muted-foreground">
                {row.curation_status !== 'unknown' ? <span className="rounded-full bg-primary/10 px-1.5 py-0.5 text-primary">{curationLabel(row.curation_status)}</span> : null}
                {!row.file_present ? <span className="rounded-full bg-destructive/10 px-1.5 py-0.5 text-destructive">Missing file</span> : null}
                {updateForRow(row) ? <span className="rounded-full bg-amber-500/10 px-1.5 py-0.5 text-amber-700 dark:text-amber-300">Update available</span> : null}
              </span>
            </button>
          </div>
        </td>;
      }
      case 'author': return <td key={column} className="px-3 py-2 text-sm">{row.author ?? 'Unknown'}</td>;
      case 'source': return <td key={column} className="px-3 py-2"><span className="rounded-full bg-primary/10 px-2 py-1 text-xs text-primary">{row.source_label}</span></td>;
      case 'size': return <td key={column} className="px-3 py-2 text-sm whitespace-nowrap">{row.file_present ? formatBytes(row.size_bytes) : 'Missing'}</td>;
      case 'installed': return <td key={column} className="px-3 py-2 text-sm whitespace-nowrap" title={row.installed_at}>{formatInstalledDate(row.installed_at)}</td>;
      case 'enabled': return <td key={column} className="px-3 py-2"><div className="flex items-center gap-2"><Switch checked={row.enabled} disabled={props.locked || pending[row.key]} onCheckedChange={() => void handleToggle(row)} aria-label={`${row.enabled ? 'Disable' : 'Enable'} ${row.display_name}`} /><span className="sr-only">{row.enabled ? 'Enabled' : 'Disabled'}</span>{!row.enabled ? <span className="rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">Disabled</span> : null}</div></td>;
      case 'actions': {
      // An available update is the single most actionable thing about a row, so
      // it belongs in Actions — not only in the optional `update_status`
      // column, which is off by default and left updates effectively hidden.
      // Rendered ONLY when an update actually exists, so the column stays quiet.
      const rowUpdate = updateForRow(row);
      const rowUpdatable = updateStatusForRow(row) === 'available' && rowUpdate;
      return <td key={column} className="px-3 py-2 text-right"><div className="flex items-center justify-end gap-1">
        {rowUpdatable ? (
          <button
            type="button"
            disabled={props.locked}
            onClick={() => props.onApplyUpdate?.(row, rowUpdate)}
            className="rounded p-1.5 text-primary hover:bg-primary/10 disabled:cursor-not-allowed disabled:opacity-50"
            title={props.locked ? 'Unlock the instance to update content.' : `Update ${row.display_name} to ${rowUpdate.latest_version}`}
            aria-label={`Update ${row.display_name} to ${rowUpdate.latest_version}`}
          >
            <ArrowUpCircle className="h-4 w-4" aria-hidden="true" />
          </button>
        ) : null}
        <button type="button" onClick={() => props.onRemove(row)} disabled={props.locked} className="rounded p-1.5 text-destructive hover:bg-destructive/10 disabled:cursor-not-allowed disabled:opacity-50" title={props.locked ? 'Unlock the instance to remove content.' : `Remove ${row.display_name}`} aria-label={`Remove ${row.display_name}`}><Trash2 className="h-4 w-4" aria-hidden="true" /></button>
        <details className="relative"><summary className="list-none rounded p-1.5 text-muted-foreground hover:bg-accent cursor-pointer" title="More actions" aria-label={`More actions for ${row.display_name}`}><MoreHorizontal className="h-4 w-4" aria-hidden="true" /></summary><div className={`absolute right-0 z-20 w-40 rounded-lg border border-border bg-card p-1 shadow-lg ${rowIndex >= visibleRows.length - 2 ? 'bottom-full mb-1' : 'top-full mt-1'}`}><button type="button" disabled={!row.resolved_path} onClick={() => props.onRevealFile?.(row)} className="w-full rounded px-2 py-1.5 text-left text-xs hover:bg-accent disabled:opacity-50">Reveal file</button><button type="button" onClick={() => void navigator.clipboard.writeText(row.filename)} className="w-full rounded px-2 py-1.5 text-left text-xs hover:bg-accent">Copy filename</button>{props.onSetCustomIcon ? <button type="button" disabled={props.locked} onClick={() => props.onSetCustomIcon?.(row)} className="w-full rounded px-2 py-1.5 text-left text-xs hover:bg-accent disabled:opacity-50">Set custom image</button> : null}{row.registry_id || row.modrinth_id || row.mod_jar_id ? <button type="button" onClick={() => props.onOpenDetails?.(row)} className="w-full rounded px-2 py-1.5 text-left text-xs hover:bg-accent">View details</button> : null}</div></details>
      </div></td>;
      }
      case 'version': return <td key={column} className="px-3 py-2 text-sm">{row.version ?? 'Unknown'}</td>;
      case 'categories': return <td key={column} className="max-w-[220px] px-3 py-2 text-xs">{row.categories.join(', ') || 'Uncategorized'}</td>;
      case 'curation': return <td key={column} className="px-3 py-2 text-xs">{curationLabel(row.curation_status)}</td>;
      case 'agora_score': return <td key={column} className="px-3 py-2 text-sm">{row.agora_score == null ? 'Unavailable' : `Agora ${row.agora_score >= 0 ? '+' : ''}${row.agora_score}`}</td>;
      case 'modrinth_downloads': return <td key={column} className="px-3 py-2 text-sm">{formatCompactNumber(row.modrinth_downloads)}</td>;
      case 'update_status': {
        const update = updateForRow(row);
        const status = updateStatusForRow(row);
        return <td key={column} className="px-3 py-2 text-sm whitespace-nowrap">
          {status === 'available' && update ? <button type="button" disabled={props.locked} onClick={() => props.onApplyUpdate?.(row, update)} className="rounded-full bg-primary/10 px-2 py-1 text-primary hover:bg-primary/20 disabled:opacity-50" title={`Update to ${update.latest_version}`}>Update available</button> : null}
          {status === 'current' ? <span className="text-muted-foreground">Up to date</span> : null}
          {status === 'unavailable' ? <span className="text-muted-foreground">Unavailable</span> : null}
          {status === 'unchecked' ? <span className="text-muted-foreground">Not checked</span> : null}
        </td>;
      }
      case 'loader_mod_id': return <td key={column} className="px-3 py-2 text-xs">Unknown</td>;
    }
  };

  return <section className="rounded-xl border border-border bg-card p-4" onDragOver={props.onDrop ? (event) => { event.preventDefault(); } : undefined} onDrop={props.onDrop}>
    <div className="mb-4 flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between">
      <div><h3 className="font-semibold text-sm">{titleForType[props.contentType]} ({props.rows.length})</h3><p className="mt-1 text-xs text-muted-foreground">Manage installed {contentLabel.toLowerCase()} without changing their safe removal workflow.</p></div>
      <div className="flex flex-wrap items-center gap-2"><button type="button" onClick={props.onAdd} disabled={props.locked} className="rounded-lg border border-input bg-background px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50" title={props.locked ? `Unlock the instance to add ${contentLabel.toLowerCase()}.` : undefined}>{props.locked ? 'Locked' : props.addLabel}</button>{props.extraActions}</div>
    </div>
    {selectedRows.length > 0 ? <div className="mb-3 flex flex-wrap items-center gap-2 rounded-lg border border-primary/20 bg-primary/5 p-2 text-xs"><span className="font-medium">{selectedRows.length} selected</span><button type="button" onClick={() => void handleBulkToggle(true)} disabled={props.locked || bulkBusy} className="rounded border border-input bg-background px-2 py-1 hover:bg-accent disabled:opacity-50">Enable selected</button><button type="button" onClick={() => void handleBulkToggle(false)} disabled={props.locked || bulkBusy} className="rounded border border-input bg-background px-2 py-1 hover:bg-accent disabled:opacity-50">Disable selected</button><button type="button" onClick={handleBulkRemove} disabled={props.locked || bulkBusy} className="rounded border border-destructive/40 bg-background px-2 py-1 text-destructive hover:bg-destructive/10 disabled:opacity-50">Remove selected</button><button type="button" onClick={() => setSelectedKeys(new Set())} className="ml-auto text-primary hover:underline">Clear selection</button></div> : null}
    <div className="flex flex-col gap-2 lg:flex-row lg:flex-wrap lg:items-center">
      <label className="relative min-w-48 flex-1"><Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" aria-hidden="true" /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search installed content…" aria-label="Search installed content" className="w-full rounded-lg border border-input bg-background py-2 pl-9 pr-3 text-sm" /></label>
      <div className="flex flex-col gap-2 lg:flex-row lg:flex-wrap lg:items-center">
        {props.onCheckUpdates ? <button type="button" onClick={() => void handleCheckUpdates()} disabled={checkingUpdates} className="rounded-lg border border-input bg-background px-3 py-2 text-sm hover:bg-accent disabled:opacity-50">{checkingUpdates ? 'Checking…' : 'Check for updates'}</button> : null}
        <details className="relative"><summary className="list-none cursor-pointer rounded-lg border border-input bg-background px-3 py-2 text-sm">Category ({filters.categories.length})</summary><div className="absolute right-0 z-20 mt-1 max-h-64 w-56 overflow-y-auto rounded-lg border border-border bg-card p-2 shadow-lg">{available.categories.length === 0 ? <span className="px-2 text-xs text-muted-foreground">No categories</span> : available.categories.map((category) => <label key={category} className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-accent"><input type="checkbox" checked={filters.categories.includes(category)} onChange={() => toggleCategory(category)} />{category}</label>)}</div></details>
        <select value={filters.curation} onChange={(event) => setFilters((current) => ({ ...current, curation: event.target.value }))} className="rounded-lg border border-input bg-background px-3 py-2 text-sm" aria-label="Curation filter"><option value="all">Curation: All</option><option value="curated">Curated</option><option value="under_review">Under review</option><option value="uncurated">Uncurated</option><option value="archived">Archived</option><option value="unknown">Unknown</option></select>
        <select value={filters.source} onChange={(event) => setFilters((current) => ({ ...current, source: event.target.value }))} className="rounded-lg border border-input bg-background px-3 py-2 text-sm" aria-label="Source filter"><option value="all">Source: All</option>{available.sources.map((source) => <option key={source} value={source}>{source}</option>)}</select>
        <select value={filters.enabled} onChange={(event) => setFilters((current) => ({ ...current, enabled: event.target.value as ContentFilters['enabled'] }))} className="rounded-lg border border-input bg-background px-3 py-2 text-sm" aria-label="Enabled state filter"><option value="all">State: All</option><option value="enabled">Enabled</option><option value="disabled">Disabled</option><option value="missing">Missing file</option></select>
        <details className="relative"><summary className="list-none cursor-pointer rounded-lg border border-input bg-background px-3 py-2 text-sm">Columns</summary><div className="absolute right-0 z-20 mt-1 w-56 rounded-lg border border-border bg-card p-2 shadow-lg">{allColumns.map((column) => <label key={column} className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-accent"><input type="checkbox" checked={columnVisible(column)} onChange={() => toggleColumn(column)} />{columnLabels[column]}</label>)}</div></details>
      </div>
    </div>
    {(filterCount > 0 || query) ? <div className="mt-3 flex flex-wrap items-center gap-2 text-xs">{query ? <span className="rounded-full bg-muted px-2 py-1">Search: {normalizeSearchText(query)} <button type="button" onClick={() => setQuery('')} aria-label="Clear search">×</button></span> : null}{filters.categories.map((category) => <button type="button" key={category} onClick={() => toggleCategory(category)} className="rounded-full bg-primary/10 px-2 py-1 text-primary">Category: {category} ×</button>)}{filters.source !== 'all' ? <button type="button" onClick={() => setFilters((current) => ({ ...current, source: 'all' }))} className="rounded-full bg-primary/10 px-2 py-1 text-primary">Source: {filters.source} ×</button> : null}{filters.curation !== 'all' ? <button type="button" onClick={() => setFilters((current) => ({ ...current, curation: 'all' }))} className="rounded-full bg-primary/10 px-2 py-1 text-primary">Curation: {curationLabel(filters.curation)} ×</button> : null}{filters.enabled !== 'all' ? <button type="button" onClick={() => setFilters((current) => ({ ...current, enabled: 'all' }))} className="rounded-full bg-primary/10 px-2 py-1 text-primary">State: {filters.enabled} ×</button> : null}<button type="button" onClick={clearFilters} className="font-medium text-primary hover:underline">Clear filters</button></div> : null}
    <div className="mt-3 flex items-center justify-between text-xs text-muted-foreground"><span>Showing {visibleRows.length} of {props.rows.length}</span>{filterCount > 0 && !query ? <button type="button" onClick={clearFilters} className="text-primary hover:underline">Clear filters</button> : null}</div>
    {props.rows.length === 0 ? <p className="mt-4 text-sm text-muted-foreground">No {contentLabel.toLowerCase()} installed.</p> : visibleRows.length === 0 ? <div className="mt-4 rounded-lg border border-dashed border-border p-6 text-center text-sm text-muted-foreground">No installed content matches your search and filters.<br /><button type="button" onClick={() => { setQuery(''); clearFilters(); }} className="mt-2 font-medium text-primary hover:underline">Clear filters</button></div> : <div className="mt-3 overflow-x-auto"><table className="w-full min-w-[800px] border-collapse text-left"><thead><tr className="border-b border-border"><th scope="col" className="sticky top-0 z-10 w-10 bg-card px-3 py-2"><input type="checkbox" checked={allVisibleSelected} onChange={toggleVisibleSelection} disabled={props.locked || visibleRows.length === 0} aria-label={allVisibleSelected ? 'Deselect visible rows' : 'Select visible rows'} /></th>{columns.map(renderHeader)}</tr></thead><tbody>{visibleRows.map((row, rowIndex) => <tr key={row.key} className={`border-b border-border last:border-0 ${!row.enabled ? 'opacity-65' : ''}`}><td className="w-10 px-3 py-2"><input type="checkbox" checked={selectedKeys.has(row.key)} onChange={() => toggleSelected(row.key)} disabled={props.locked} aria-label={`Select ${row.display_name}`} /></td>{columns.map((column) => renderCell(row, column, rowIndex))}</tr>)}</tbody></table></div>}
  </section>;
}
