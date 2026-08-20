import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Check, ChevronDown, Download, Leaf, List, LayoutGrid, Search } from 'lucide-react';
import {
  browseSearch,
  browseLoadMore,
  forYouItems,
  formatError,
  getInstanceDetail,
  getSetting,
  listCategories,
  listInstances,
  listManifestLoaders,
  listManifestMcVersions,
  listModrinthCategories,
  type BrowseItemCached,
  type CategoryInfo,
  type RegistryItem,
  type SortOption,
  type ModrinthSearchResult,
  type ModrinthCategoryInfo,
  type InstanceDetail,
  type InstanceRow,
} from '../lib/tauri';
import { useRegistryState } from '../lib/useRegistryState';
import { loadPreference } from '../features/interactive/live/presentationPreference';
import { RegistryStatusView } from '../components/registry-status-view';
import { BrowseListResults } from '../components/browse/BrowseListResults';
import { BrowseTileResults } from '../components/browse/BrowseTileResults';
import { BrowseBazaar } from '../components/browse/BrowseBazaar';
import { presentationCapabilities } from '../components/presentation-capabilities';
import type { BazaarItem } from '../components/browse/bazaar-model';
import { InstallFlow } from '../components/InstallFlow';
import { usePackInstall } from '../components/PackInstallProgress';
import { showToast } from '../components/Toast';
import {
  saveBrowseSnapshot,
  peekBrowseSnapshot,
  parkBrowseSnapshot,
  takeBrowseSnapshot,
  clearBrowseSnapshot,
  type BrowseSnapshot,
} from './browseSession';
import {
  resolveInstallPlan,
  type BatchInstallItem,
  type InstallIntent,
  type ResolvedInstallPlan,
  planNeedsUserReview,
} from '../lib/installFlow';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from '../components/ui/dialog';
import type { BrowseItemContext as ItemContext } from '../components/browse/types';
import '../components/browse/browse-cards.css';
import { agoraRepositoryUrl } from '../lib/brandConfig';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../components/ui/dropdown-menu';

function parseBool(raw: unknown): boolean {
  return raw === true || raw === 'true' || raw === 1 || raw === '1';
}

/**
 * Whether live Modrinth *browsing* is on.
 *
 * This is the third-party discovery axis only, and it is off by default —
 * curated-only is the default state, not a mode. It says nothing about curated
 * catalog entries that happen to be Modrinth-sourced: those are governed by
 * `curated_source_modrinth_id_enabled` and stay browsable and installable here.
 */
async function modrinthEffectivelyEnabled(): Promise<boolean> {
  const mr = await Promise.allSettled([getSetting('modrinth_enabled')]);
  return mr[0].status === 'fulfilled' && parseBool(mr[0].value);
}

function useDebounce<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(id);
  }, [value, delay]);
  return debounced;
}

// `net_score` now means the blended 0-100 cross-source score, not the raw
// upvotes-minus-downvotes tally it originally referred to. The values are kept
// so saved preferences and query keys stay valid.
const SORTS: { label: string; value: SortOption }[] = [
  { label: 'For You', value: 'for_you' },
  { label: 'Best', value: 'net_score' },
  { label: 'Trending', value: 'velocity' },
  { label: 'Newest', value: 'newest' },
  { label: 'Most Endorsed', value: 'most_upvoted' },
  { label: 'Lowest Rated', value: 'most_downvoted' },
];

/**
 * Simple mode's sort list: the same orders, minus "For You".
 *
 * "For You" is the taste model — it watches what you install and reorders the
 * catalogue around it. Simple mode does not profile the user, so it drops the
 * option entirely and lands on `net_score` ("Best"), the plain cross-source
 * score. The preference is NOT rewritten when it is dropped: a saved `for_you`
 * survives untouched and comes back the moment the mode does.
 */
const SIMPLE_SORTS = SORTS.filter((s) => s.value !== 'for_you');
const SIMPLE_DEFAULT_SORT: SortOption = 'net_score';

const CONTENT_TYPES = ['mod', 'pack', 'shader', 'resourcepack', 'server', 'datapack', 'world'];

const MODRINTH_PROJECT_TYPES: Partial<Record<string, string>> = {
  mod: 'mod',
  pack: 'modpack',
  shader: 'shader',
  resourcepack: 'resourcepack',
  server: 'minecraft_java_server',
  datapack: 'datapack',
};

const SUPPORTED_MODRINTH_PROJECT_TYPES = new Set(Object.values(MODRINTH_PROJECT_TYPES));

/** Bazaar stall id ⇄ Browse content type. `null` content type = "Everything". */
function contentTypeToStall(contentType: string | null): string {
  return contentType ?? 'all';
}

function formatCategoryName(value: string): string {
  return value.replace(/[-_]/g, ' ').replace(/\b\w/g, (character) => character.toUpperCase());
}

function modrinthCategoryMatchesContentType(category: ModrinthCategoryInfo, contentType: string | null): boolean {
  if (contentType === null) return SUPPORTED_MODRINTH_PROJECT_TYPES.has(category.project_type);
  return MODRINTH_PROJECT_TYPES[contentType] === category.project_type;
}

function curatedCategoryMatchesContentType(category: CategoryInfo, contentType: string | null): boolean {
  return contentType === null || category.content_types.includes(contentType);
}

function CuratedCategoryDropdown({
  categories,
  selectedCategory,
  onSelect,
  disabled,
}: {
  categories: CategoryInfo[];
  selectedCategory: string | null;
  onSelect: (category: string | null) => void;
  disabled: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const selected = categories.find((category) => category.id === selectedCategory) ?? null;
  const normalizedSearch = search.trim().toLocaleLowerCase();
  const filteredCategories = normalizedSearch
    ? categories.filter((category) =>
        category.display_name.toLocaleLowerCase().includes(normalizedSearch)
        || category.id.toLocaleLowerCase().includes(normalizedSearch),
      )
    : categories;

  return (
    <DropdownMenu
      open={open}
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen);
        if (!nextOpen) setSearch('');
      }}
    >
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          aria-label={selected ? `Curated category: ${selected.display_name}` : 'Curated categories'}
          className={[
            'inline-flex min-w-48 items-center justify-between gap-2 rounded-lg border px-3 py-2 text-sm transition-colors disabled:cursor-not-allowed disabled:opacity-50',
            selected
              ? 'border-primary bg-primary text-primary-foreground'
              : 'border-input bg-background hover:bg-accent',
          ].join(' ')}
        >
          <span className="truncate">{selected ? `Curated: ${selected.display_name}` : 'Curated categories'}</span>
          <ChevronDown aria-hidden className="h-4 w-4 shrink-0" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-72 p-2">
        <div
          className="relative mb-2"
          onKeyDown={(event) => event.stopPropagation()}
        >
          <Search aria-hidden className="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <input
            autoFocus
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search curated categories…"
            aria-label="Search curated categories"
            className="w-full rounded-md border border-input bg-background py-2 pl-8 pr-3 text-sm outline-none focus:ring-2 focus:ring-ring"
          />
        </div>
        <div className="max-h-64 overflow-y-auto">
          <DropdownMenuItem onSelect={() => onSelect(null)}>
            <span className="flex-1">All categories</span>
            {selectedCategory === null && <Check aria-hidden className="h-4 w-4" />}
          </DropdownMenuItem>
          {filteredCategories.map((category) => (
            <DropdownMenuItem key={category.id} onSelect={() => onSelect(category.id)}>
              <span className="flex-1 truncate">{category.display_name}</span>
              {category.id === selectedCategory && <Check aria-hidden className="h-4 w-4" />}
            </DropdownMenuItem>
          ))}
          {filteredCategories.length === 0 && (
            <p className="px-2 py-3 text-center text-xs text-muted-foreground">No curated categories found.</p>
          )}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

type BrowseItem = BrowseItemCached;

function parseHttpsGallery(json: string | null): string[] {
  if (!json) return [];
  try {
    const values = JSON.parse(json);
    if (!Array.isArray(values)) return [];
    return values.filter((value): value is string => {
      if (typeof value !== 'string') return false;
      try { return new URL(value).protocol === 'https:'; } catch { return false; }
    });
  } catch {
    return [];
  }
}

function registryVersions(item: RegistryItem): string[] {
  if (!item.compatible_versions_json) return [];
  try {
    const entries = JSON.parse(item.compatible_versions_json);
    if (!Array.isArray(entries)) return [];
    return [...new Set(entries.map((entry) => entry?.mc_version).filter((value): value is string => typeof value === 'string'))];
  } catch {
    return [];
  }
}

function presentationFields(registryItem: RegistryItem | null, modrinthResult: ModrinthSearchResult | null) {
  const registryGallery = registryItem ? parseHttpsGallery(registryItem.gallery_urls_json) : [];
  const featuredGallery = modrinthResult?.featured_gallery;
  const heroImageUrl = featuredGallery && featuredGallery.startsWith('https://')
    ? featuredGallery
    : registryGallery[0] ?? modrinthResult?.gallery?.find((url) => url.startsWith('https://')) ?? null;
  const supportedVersions = registryItem ? registryVersions(registryItem) : [];
  return {
    heroImageUrl,
    author: modrinthResult?.author || null,
    categories: modrinthResult?.categories ?? [],
    downloads: modrinthResult?.downloads ?? null,
    follows: modrinthResult?.follows ?? null,
    upvotes: registryItem?.upvotes ?? null,
    downvotes: registryItem?.downvotes ?? null,
    netScore: registryItem?.net_score ?? null,
    supportedVersions: supportedVersions.length > 0 ? supportedVersions : modrinthResult?.versions ?? [],
    sourcePageUrl: registryItem?.page_url
      ?? (modrinthResult?.slug ? `https://modrinth.com/${modrinthResult.project_type}/${modrinthResult.slug}` : null),
  };
}

function failedBatchRootItemId(plan: ResolvedInstallPlan, error: string): string | undefined {
  if (plan.intent.action.type !== 'batch-install') return undefined;
  const filename = error.match(/verification failed for ([^:]+):/i)?.[1]?.trim();
  if (!filename) return undefined;
  const rootIds = new Set(plan.intent.action.items.map((item) => item.itemId));
  const find = (operation: ResolvedInstallPlan['operation']): string | undefined => {
    switch (operation.type) {
      case 'install':
        return operation.artifact.filename === filename && rootIds.has(operation.artifact.itemId)
          ? operation.artifact.itemId
          : undefined;
      case 'update':
        return operation.newArtifact.filename === filename && rootIds.has(operation.newArtifact.itemId)
          ? operation.newArtifact.itemId
          : undefined;
      case 'batch-install':
      case 'batch-update':
      case 'reconcile':
        return operation.operations.map(find).find((itemId): itemId is string => Boolean(itemId));
      default:
        return undefined;
    }
  };
  return find(plan.operation);
}

/**
 * Shape registry rows into browse items for the For You path.
 *
 * The full curated/Modrinth/Technic merge and ranking lives in core
 * (`browse_cache::merge_items`); this only covers For You, which is
 * registry-only and ranked by category overlap rather than by score.
 */
function curatedOnlyItems(registryItems: RegistryItem[]): BrowseItem[] {
  return registryItems.map((ri) => ({
    id: ri.id,
    source: 'curated',
    registryItem: ri,
    modrinthResult: null,
    name: ri.name,
    iconUrl: ri.icon_url,
    description: ri.description,
    contentType: ri.content_type,
    ...presentationFields(ri, null),
  }));
}

// ---------------------------------------------------------------------------
// Deterministic query-key — stable string identity for the current filter set.
// ---------------------------------------------------------------------------
function computeQueryKey(params: {
  sort: SortOption;
  category: string | null;
  contentType: string | null;
  mcVersion: string | null;
  loader: string | null;
  query: string;
}): string {
  return JSON.stringify({
    sort: params.sort,
    category: params.category,
    contentType: params.contentType,
    mcVersion: params.mcVersion,
    loader: params.loader,
    query: params.query,
  });
}

// ---- Registry recovery shell (no hooks after this point) ----
function RegistryRecoveryShell({
  state,
  status,
  error,
  actions,
}: {
  state: import('../lib/useRegistryState').RegistryState;
  status: import('../lib/tauri').RegistryStatus | null;
  error: string | null;
  actions: import('../lib/useRegistryState').RegistryActions;
}) {
  return (
    <div className="space-y-6">
      <section className="agora-hero compact">
        <h2 className="text-2xl font-bold mb-2">Browse</h2>
        <p className="text-muted-foreground">
          Curated mods, packs, shaders, resource packs, and more.
        </p>
      </section>
      <RegistryStatusView
        variant="fullscreen"
        state={state}
        status={status}
        error={error}
        actions={actions}
      />
    </div>
  );
}

export function Browse({ onSelectMod, onOpenInstance, initialInstanceId, initialContentType }: { onSelectMod?: (id: string, instanceId?: string) => void; onOpenInstance?: (instanceId: string) => void; initialInstanceId?: string; initialContentType?: string }) {
  // Registry availability — show recovery panel when missing.
  // This is the ONLY hook call in this component, so the hook count is stable.
  const registry = useRegistryState();

  // Bail out BEFORE any other hooks when the registry is unavailable.
  //
  // Conditions for showing the recovery shell:
  //   - State is exactly 'missing' (no cache, no loading)
  //   - State is 'loading' AND no cached database exists (registry download in
  //     progress but we have nothing to show)
  //   - State is 'unknown' with an error (status-read failed)
  //
  // Once the user has a cached database they can browse cached content, even
  // while an update downloads in the background ('loading' with cache).
  const showRecovery =
    registry.state === 'missing' ||
    (registry.state === 'loading' && !registry.hasCachedDb) ||
    (registry.state === 'unknown' && registry.error !== null);

  if (showRecovery) {
    return (
      <RegistryRecoveryShell
        state={registry.state}
        status={registry.status}
        error={registry.error}
        actions={registry.actions}
      />
    );
  }

  return <BrowseContent onSelectMod={onSelectMod} onOpenInstance={onOpenInstance} initialInstanceId={initialInstanceId} initialContentType={initialContentType} registryState={registry.state} registryStatus={registry.status} registryError={registry.error} registryActions={registry.actions} />;
}

function BrowseContent({
  onSelectMod,
  onOpenInstance,
  initialInstanceId,
  initialContentType,
  registryState: regState,
  registryStatus: regStatus,
  registryError: regError,
  registryActions: regActions,
}: {
  onSelectMod?: (id: string, instanceId?: string) => void;
  onOpenInstance?: (instanceId: string) => void;
  initialInstanceId?: string;
  initialContentType?: string;
  registryState: import('../lib/useRegistryState').RegistryState;
  registryStatus: import('../lib/tauri').RegistryStatus | null;
  registryError: string | null;
  registryActions: import('../lib/useRegistryState').RegistryActions;
}) {
  // ---- Filter state ----
  const [items, setItems] = useState<BrowseItemCached[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [categories, setCategories] = useState<CategoryInfo[]>([]);
  const [modrinthCategories, setModrinthCategories] = useState<ModrinthCategoryInfo[]>([]);
  /**
   * Which Browse surface this interaction mode uses. Read once per mount, like
   * every other presentation read on this page: `bazaar` is the High
   * Interaction shelf, `simple` is this page with the discovery extras removed,
   * `standard` is this page in full.
   */
  const browseSurface = useMemo(() => {
    try {
      return presentationCapabilities(loadPreference()).browse;
    } catch {
      return 'standard' as const;
    }
  }, []);
  const simpleBrowse = browseSurface === 'simple';
  const sortOptions = simpleBrowse ? SIMPLE_SORTS : SORTS;

  const [sort, setSort] = useState<SortOption>(() => {
    try {
      const saved = localStorage.getItem('browse_sort');
      if (saved && sortOptions.some((s) => s.value === saved)) return saved as SortOption;
    } catch { /* ignore */ }
    return simpleBrowse ? SIMPLE_DEFAULT_SORT : 'net_score';
  });
  const [category, setCategory] = useState<string | null>(null);
  const [contentType, setContentType] = useState<string | null>(initialContentType ?? 'mod');
  const [mcVersion, setMcVersion] = useState<string | null>(null);
  const [loader, setLoader] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const debouncedQuery = useDebounce(query, 250);
  const [layout, setLayout] = useState<'list' | 'grid'>(() => {
    try {
      const saved = localStorage.getItem('browse_layout');
      if (saved === 'list' || saved === 'grid') return saved;
    } catch { /* ignore */ }
    return 'list';
  });

  // High Interaction Browse (the Bazaar, v5-browse port): default on when the
  // presentation preference is High Interaction, switchable at any time.
  const [bazaarMode, setBazaarMode] = useState<boolean>(() => browseSurface === 'bazaar');
  // Scroll offset to reapply after a restored list paints. `null` means this
  // mount did not restore anything, so scrolling must be left alone.
  const restoredScrollRef = useRef<number | null>(null);
  // Bazaar shelf order, mirrored here so it can be snapshotted with the list.
  const bazaarSettledOrderRef = useRef<string[]>([]);
  /**
   * The open Bazaar stall, mapped onto the same `contentType` the Browse list
   * queries with. A stall is a different QUERY, not a filter over the loaded
   * page: previously only `mod` was ever fetched, so every other stall showed an
   * empty shelf. "Everything" clears the content type entirely.
   */
  const [bazaarStall, setBazaarStallState] = useState<string>(() => contentTypeToStall(initialContentType ?? 'mod'));
  const setBazaarStall = useCallback((stallId: string) => {
    setBazaarStallState(stallId);
    setContentType(stallId === 'all' ? null : stallId);
    setCategory(null);          // a category from the old stall cannot apply to the new one
  }, []);

  // ---- Separate loading/error state for metadata vs search ----
  const [metaLoading, setMetaLoading] = useState(true);
  const [metaError, setMetaError] = useState<string | null>(null);
  const [searchLoading, setSearchLoading] = useState(true);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [loadMoreLoading, setLoadMoreLoading] = useState(false);
  const [loadMoreError, setLoadMoreError] = useState<string | null>(null);
  // Tracks the 0-indexed page displayed. Starts at 0; incremented after each
  // successful load-more. Reset to 0 on a new search.
  const [currentPage, setCurrentPage] = useState(0);

  // ---- Request generation counter ----
  const generationRef = useRef(0);
  const inFlightSearchRef = useRef<number | null>(null);
  const inFlightLoadMoreRef = useRef<number | null>(null);

  const [loaders, setLoaders] = useState<string[]>([]);
  const [mcVersions, setMcVersions] = useState<string[]>([]);
  const [instances, setInstances] = useState<InstanceRow[]>([]);
  const [activeInstanceId, setActiveInstanceId] = useState('');
  const [activeInstance, setActiveInstance] = useState<InstanceDetail | null>(null);
  const [contextLoading, setContextLoading] = useState(false);
  const [contextError, setContextError] = useState<string | null>(null);
  const [autoConfirmCleanInstalls, setAutoConfirmCleanInstalls] = useState(false);
  const [alwaysAutoConfirmInstalls, setAlwaysAutoConfirmInstalls] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void Promise.all([
      getSetting('install_auto_confirm_clean'),
      getSetting('install_always_auto_confirm'),
    ])
      .then(([autoConfirm, alwaysConfirm]) => {
        if (!cancelled) {
          setAutoConfirmCleanInstalls(parseBool(autoConfirm));
          setAlwaysAutoConfirmInstalls(parseBool(alwaysConfirm));
        }
      })
      .catch(() => {
        // Missing or unavailable settings fail closed: keep the review screen.
      });
    return () => { cancelled = true; };
  }, []);

  // ---- Multi-select bulk install ----
  // Keyed by `${source}:${id}`; the map holds the item so the selection
  // survives list refreshes (load-more, retries) between toggling and install.
  const [selectedItems, setSelectedItems] = useState<Map<string, BrowseItem>>(new Map());
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerInstanceId, setPickerInstanceId] = useState('');
  const [installTarget, setInstallTarget] = useState<{
    intent: InstallIntent;
    instanceName: string;
    initialPlan?: ResolvedInstallPlan | null;
    initialError?: { message: string; code?: string; skipItemId?: string } | null;
  } | null>(null);
  const { startResolvingPlan, startPlan } = usePackInstall();
  // Guards against double-clicks spawning concurrent installs while a batch
  // attempt is resolving.
  const bulkInstallBusyRef = useRef(false);

  const clearSelection = useCallback(() => setSelectedItems(new Map()), []);

  const toggleSelect = useCallback((item: BrowseItem) => {
    setSelectedItems((current) => {
      const next = new Map(current);
      const key = `${item.source}:${item.id}`;
      if (next.has(key)) next.delete(key);
      else next.set(key, item);
      return next;
    });
  }, []);

  // Installed items cannot be bulk-selected: re-installing them is a
  // per-mod decision made from the details page (with version choice).
  const handleToggleSelect = (item: BrowseItem) => {
    if (contextFor(item)?.installed) {
      showToast(
        `${item.name} is already installed in ${activeInstance?.row.name ?? 'this instance'}.`,
        'error',
      );
      return;
    }
    toggleSelect(item);
  };

  const buildBatchInstallIntent = (instanceId: string, items: Iterable<BrowseItem>): InstallIntent => {
    const batchItems: BatchInstallItem[] = [...items].map((item) => ({
      sourceType: item.source === 'curated' ? 'curated' : 'modrinth',
      itemId: item.id,
    }));
    return {
      action: { type: 'batch-install', items: batchItems },
      targetInstance: instanceId,
      // Resolve the default bulk behavior during background preparation. The
      // review still shows optional rows unchecked, but confirming without
      // opting in no longer causes a second review cycle.
      optionalDeps: { type: 'exclude-all' },
      requestedBy: 'interactive',
      overrides: {
        allowReplace: false,
        skipHealthScan: false,
        forceConflictResolution: {},
      },
    };
  };

  const bulkInstallLabel = (count: number) => `Installing ${count} selected item${count === 1 ? '' : 's'}`;

  // `sourceItems` lets callers (e.g. the Bazaar's staged bag) bulk-install a
  // specific batch; the default is the current multi-select.
  const startBulkInstall = (instanceId: string, sourceItems?: Iterable<BrowseItem>) => {
    if (bulkInstallBusyRef.current) return;
    bulkInstallBusyRef.current = true;
    const batch = Array.from(sourceItems ?? selectedItems.values());
    if (batch.length === 0) {
      bulkInstallBusyRef.current = false;
      return;
    }
    const instance = instances.find((candidate) => candidate.instance_id === instanceId);
    const instanceName = instance?.name ?? instanceId;
    const intent = buildBatchInstallIntent(instanceId, batch);
    const count = batch.length;
    const label = bulkInstallLabel(count);

    // Resolve in the corner first, then promote anything needing attention to
    // the focused review dialog. Clean plans only skip review when the user
    // explicitly enabled auto-confirm in Settings.
    const resolving = startResolvingPlan(label, instanceName);
    const promoteApplyFailure = (failedPlan: ResolvedInstallPlan, error: string) => {
      resolving.dismiss();
      bulkInstallBusyRef.current = false;
      setInstallTarget({
        intent,
        instanceName,
        initialPlan: failedPlan,
        initialError: {
          message: error,
          code: 'ERR_APPLY',
          skipItemId: failedBatchRootItemId(failedPlan, error),
        },
      });
    };
    void resolveInstallPlan(intent)
      .then((plan) => {
        const needsAttention = !autoConfirmCleanInstalls || planNeedsUserReview(plan, {
          ignoreDependencies: alwaysAutoConfirmInstalls,
        });
        if (needsAttention) {
          resolving.dismiss();
          bulkInstallBusyRef.current = false;
          setInstallTarget({ intent, instanceName, initialPlan: plan });
        } else {
          clearSelection();
          resolving.apply(plan, (error, failedPlan) => promoteApplyFailure(failedPlan, error));
        }
      })
      .catch(() => {
        bulkInstallBusyRef.current = false;
        resolving.dismiss();
        // Keep resolution failures actionable: the focused flow offers retry
        // and close instead of hiding the error in a toast.
        setInstallTarget({ intent, instanceName });
      });
  };

  // Technic packs have no registry id and no detail page, so selecting one
  // installs it directly instead of routing through `onSelectMod`. The
  // `technic:` prefix is applied in core when the item is built.
  // Technic packs now have a detail page of their own (ModDetail resolves a
  // `technic:<slug>` id through technicPackDetail), so selecting a card opens
  // details like every other source instead of installing immediately.
  const handleSelectItem = (id: string) => {
    // Capture where the user was before the detail page replaces this view.
    // App also saves `main`'s scrollTop for the standard list, but the Bazaar
    // reaches the detail page through the same handler and was never covered.
    parkBrowseSnapshot(document.querySelector('main')?.scrollTop ?? 0);
    onSelectMod?.(id, activeInstanceId || undefined);
  };

  const handleInstallSelected = () => {
    if (selectedItems.size === 0) return;
    if (activeInstanceId) {
      startBulkInstall(activeInstanceId);
    } else {
      setPickerInstanceId('');
      setPickerOpen(true);
    }
  };

  const confirmPickerInstall = () => {
    if (!pickerInstanceId) return;
    setPickerOpen(false);
    startBulkInstall(pickerInstanceId);
  };

  const installBarTargetName = activeInstance?.row.name
    ?? instances.find((candidate) => candidate.instance_id === activeInstanceId)?.name;

  const visibleCuratedCategories = useMemo(
    () => categories.filter((item) => curatedCategoryMatchesContentType(item, contentType)),
    [categories, contentType],
  );
  const visibleModrinthCategories = useMemo(() => {
    const seen = new Set<string>();
    return modrinthCategories
      .filter((item) => modrinthCategoryMatchesContentType(item, contentType))
      .filter((item) => {
        if (seen.has(item.name)) return false;
        seen.add(item.name);
        return true;
      })
      .sort((left, right) => formatCategoryName(left.name).localeCompare(formatCategoryName(right.name)));
  }, [contentType, modrinthCategories]);

  useEffect(() => {
    let cancelled = false;
    void listInstances()
      .then((result) => { if (!cancelled) setInstances(result); })
      .catch((cause) => { if (!cancelled) setContextError(formatError(cause)); });
    return () => { cancelled = true; };
  }, []);

  const selectInstanceContext = async (instanceId: string) => {
    setActiveInstanceId(instanceId);
    setContextError(null);
    setActiveInstance(null);
    if (!instanceId) return;
    setContextLoading(true);
    try {
      const detail = await getInstanceDetail(instanceId);
      if (!detail) throw new Error('The selected instance no longer exists.');
      setActiveInstance(detail);
      setMcVersion(detail.row.minecraft_version);
      setLoader(detail.row.loader);
    } catch (cause) {
      setContextError(formatError(cause));
    } finally {
      setContextLoading(false);
    }
  };

  useEffect(() => {
    if (!initialInstanceId || activeInstanceId === initialInstanceId) return;
    if (!instances.some((instance) => instance.instance_id === initialInstanceId)) return;
    void selectInstanceContext(initialInstanceId);
  }, [activeInstanceId, initialInstanceId, instances]);

  const handleSortChange = (next: SortOption) => {
    setSort(next);
    if (next === 'for_you') setCategory(null);
    try { localStorage.setItem('browse_sort', next); } catch { /* ignore */ }
  };

  // ---- Clear all filters ----
  const clearFilters = useCallback(() => {
    setCategory(null);
    setContentType(null);
    setMcVersion(activeInstance?.row.minecraft_version ?? null);
    setLoader(activeInstance?.row.loader ?? null);
    setQuery('');
    setSearchError(null);
    generationRef.current += 1;
  }, [activeInstance]);

  const hasActiveFilters = category !== null || contentType !== null || mcVersion !== null || loader !== null || query.trim() !== '';

  const contextFor = (item: BrowseItem): ItemContext | null => {
    if (!activeInstance) return null;
    const installed = activeInstance.manifest
      ? [
          ...activeInstance.manifest.mods,
          ...activeInstance.manifest.resourcepacks,
          ...activeInstance.manifest.shaders,
          ...activeInstance.manifest.datapacks,
          ...activeInstance.manifest.worlds,
        ].some((entry) =>
          entry.registry_id === item.id
          || entry.modrinth_id === item.id
          || entry.mod_jar_id === item.id
        )
      : false;
    return {
      installed,
      whyRecommended: sort === 'for_you'
        ? item.registryItem?.recommendation_reason
          ?? `Recommended by Agora's curated score for ${activeInstance.row.loader} ${activeInstance.row.minecraft_version}.`
        : null,
    };
  };

  // ---- The Bazaar (High Interaction Browse): owned set + item mapping ----
  const bazaarOwnedIds = useMemo(() => {
    if (!activeInstance?.manifest) return new Set<string>();
    return new Set(
      [
        ...activeInstance.manifest.mods,
        ...activeInstance.manifest.resourcepacks,
        ...activeInstance.manifest.shaders,
        ...activeInstance.manifest.datapacks,
        ...activeInstance.manifest.worlds,
      ].flatMap((entry) => [entry.registry_id, entry.modrinth_id, entry.mod_jar_id].filter((id): id is string => Boolean(id))),
    );
  }, [activeInstance?.manifest]);

  const bazaarItems = useMemo<BazaarItem[]>(() => items.map((it) => ({
    id: it.id,
    name: it.name,
    iconUrl: it.iconUrl,
    description: it.description,
    contentType: it.contentType,
    author: it.author,
    categories: it.categories ?? [],
    supportedVersions: it.supportedVersions ?? [],
    curated: it.source === 'curated',
    downloads: it.downloads ?? undefined,
    follows: it.follows ?? undefined,
  })), [items]);

  const bazaarInstanceVersion = activeInstance?.row.minecraft_version ?? null;

  // The Bazaar's "N picked" button IS the bulk-install button: it hands the
  // staged bag to the exact same reviewed batch pipeline as the standard
  // selection bar (same intent, same review, same auto-confirm rules).
  const handleBazaarInstall = useCallback((staged: BazaarItem[]) => {
    const batch: BrowseItem[] = [];
    for (const bi of staged) {
      const found = items.find((it) => it.id === bi.id);
      if (found) batch.push(found);
    }
    if (batch.length === 0) return;
    setSelectedItems(new Map(batch.map((it) => [`${it.source}:${it.id}`, it])));
    if (activeInstanceId) {
      startBulkInstall(activeInstanceId, batch);
    } else {
      setPickerInstanceId('');
      setPickerOpen(true);
    }
  }, [items, activeInstanceId, startBulkInstall]);

  // ---- Build a stable query key from active filter params ----
  const queryKey = useMemo(
    () =>
      computeQueryKey({
        sort,
        category,
        contentType,
        mcVersion,
        loader,
        query: debouncedQuery,
      }),
    [sort, category, contentType, mcVersion, loader, debouncedQuery],
  );

  // Clear the selection whenever the filter set changes so stale selections
  // never survive into a different result set. Pagination (load-more) keeps
  // the selection because it leaves the query key unchanged.
  useEffect(() => {
    setSelectedItems(new Map());
  }, [queryKey]);

  // ---- Load category metadata — separate from search ----
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        if (!cancelled) setMetaLoading(true);
        setMetaError(null);
        const [curatedResult, modrinthEnabledResult] = await Promise.allSettled([
          listCategories(),
          modrinthEffectivelyEnabled(),
        ]);
        if (cancelled) return;

        if (curatedResult.status === 'fulfilled') {
          setCategories(Array.isArray(curatedResult.value) ? curatedResult.value : []);
        } else {
          setMetaError(`Curated categories: ${formatError(curatedResult.reason)}`);
        }

        const modrinthEnabled = modrinthEnabledResult.status === 'fulfilled'
          && modrinthEnabledResult.value === true;
        if (modrinthEnabled) {
          try {
            const result = await listModrinthCategories();
            if (!cancelled) setModrinthCategories(Array.isArray(result) ? result : []);
          } catch (error) {
            if (!cancelled) {
              setMetaError((current) => [current, `Modrinth categories: ${formatError(error)}`].filter(Boolean).join(' '));
            }
          }
        } else {
          setModrinthCategories([]);
        }
      } catch (e) {
        if (!cancelled) setMetaError(formatError(e));
      } finally {
        if (!cancelled) setMetaLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (metaLoading || category === null) return;
    const isAvailable = visibleCuratedCategories.some((item) => item.id === category)
      || visibleModrinthCategories.some((item) => item.name === category);
    if (!isAvailable) setCategory(null);
  }, [category, metaLoading, visibleCuratedCategories, visibleModrinthCategories]);

  // ---- Load loaders and MC versions (static metadata) ----
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [l, v] = await Promise.all([listManifestLoaders(), listManifestMcVersions()]);
        if (!cancelled) {
          setLoaders(l);
          setMcVersions(v);
        }
      } catch {
        // degraded behavior
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Mirror the live list into the session snapshot. Kept as an effect rather
  // than written at each mutation site so no future code path can update the
  // list and forget to persist it.
  useEffect(() => {
    if (searchLoading) return;
    const snapshot: BrowseSnapshot = {
      queryKey,
      items,
      hasMore,
      currentPage,
      // Scroll is captured separately on the way out; preserve any prior value.
      scrollTop: peekBrowseSnapshot()?.scrollTop ?? 0,
      bazaarMode,
      bazaarSettledOrder: bazaarSettledOrderRef.current,
    };
    saveBrowseSnapshot(snapshot);
  }, [queryKey, items, hasMore, currentPage, bazaarMode, searchLoading]);

  // Reapply the saved scroll offset once the restored list has painted.
  useEffect(() => {
    const target = restoredScrollRef.current;
    if (target == null || items.length === 0) return;
    restoredScrollRef.current = null;
    const raf = requestAnimationFrame(() => {
      const scroller = document.querySelector('main');
      scroller?.scrollTo({ top: target, behavior: 'auto' });
    });
    return () => cancelAnimationFrame(raf);
  }, [items]);

  // ---- Main search effect — uses generation counter for stale-response protection ----
  useEffect(() => {
    // Returning from a mod detail page with the same filters restores the
    // already-loaded pages instead of refetching page 0, so the user lands
    // back where they were rather than at the top of a one-page list.
    const restored = takeBrowseSnapshot(queryKey);
    if (restored) {
      restoredScrollRef.current = restored.scrollTop;
      bazaarSettledOrderRef.current = restored.bazaarSettledOrder;
      setItems(restored.items);
      setHasMore(restored.hasMore);
      setCurrentPage(restored.currentPage);
      // A snapshot taken in High Interaction must not drag the Bazaar back into
      // Simple mode after the preference changed underneath it.
      setBazaarMode(restored.bazaarMode && !simpleBrowse);
      setSearchLoading(false);
      setSearchError(null);
      return;
    }

    // Different filters — the cached list no longer matches the controls.
    clearBrowseSnapshot();
    const generation = ++generationRef.current;
    inFlightSearchRef.current = generation;
    setSearchError(null);
    setSearchLoading(true);
    // Reset items and pagination state immediately so stale results
    // and stuck load-more states are never visible.
    setItems([]);
    setHasMore(false);
    setLoadMoreLoading(false);
    setLoadMoreError(null);
    setCurrentPage(0);
    inFlightLoadMoreRef.current = null;

    let cancelled = false;

    (async () => {
      try {
        if (sort === 'for_you') {
          const registryItems = await forYouItems(
            mcVersion ?? undefined,
            loader ?? undefined,
            undefined,
            undefined,
            debouncedQuery.trim() || undefined,
          );

          if (!cancelled && inFlightSearchRef.current === generation) {
            setItems(curatedOnlyItems(registryItems));
            setHasMore(false);
          }
        } else {
          const page = await browseSearch(
            queryKey,
            debouncedQuery.trim() || undefined,
            contentType ?? undefined,
            category ?? undefined,
            sort,
            mcVersion ?? undefined,
            loader ?? undefined,
          );

          if (!cancelled && inFlightSearchRef.current === generation) {
            setItems(page.items);
            setHasMore(page.hasMore);
          }
        }
      } catch (e) {
        if (!cancelled && inFlightSearchRef.current === generation) {
          setSearchError(formatError(e));
          setItems([]);
          setHasMore(false);
        }
      } finally {
        if (!cancelled && inFlightSearchRef.current === generation) {
          setSearchLoading(false);
          inFlightSearchRef.current = null;
        }
      }
    })();

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [queryKey]);

  const loadNextPage = useCallback(() => {
    if (!hasMore || searchLoading || loadMoreLoading) return;
    const capturedGeneration = generationRef.current;
    inFlightLoadMoreRef.current = capturedGeneration;
    const nextPage = currentPage + 1;
    setLoadMoreLoading(true);
    setLoadMoreError(null);

    browseLoadMore(queryKey, nextPage)
      .then((page) => {
        if (capturedGeneration === generationRef.current) {
          setItems((prev) => {
            const existingKeys = new Set(prev.map((i) => `${i.id}:${i.source}`));
            const newItems = page.items.filter(
              (ni) => !existingKeys.has(`${ni.id}:${ni.source}`),
            );
            return newItems.length === 0 ? prev : [...prev, ...newItems];
          });
          setHasMore(page.hasMore);
          setCurrentPage(nextPage);
        }
      })
      .catch((e) => {
        if (
          capturedGeneration === generationRef.current
          && !(typeof e === 'object' && e !== null && 'code' in e && e.code === 'ERR_BROWSE_STALE')
        ) {
          setLoadMoreError(formatError(e));
        }
      })
      .finally(() => {
        if (capturedGeneration === generationRef.current) {
          setLoadMoreLoading(false);
          inFlightLoadMoreRef.current = null;
        }
      });
  }, [currentPage, hasMore, loadMoreLoading, queryKey, searchLoading]);

  // ---- Infinite scroll ----
  //
  // Attached via a CALLBACK REF rather than an effect keyed on a dependency
  // list. The sentinel is only rendered in list/tile mode, so returning from
  // the Bazaar mounts it without changing any of hasMore/searchLoading/
  // loadMoreLoading/loadMoreError — an effect keyed on those never re-ran, its
  // `sentinelRef.current` was still null from Bazaar mode, and no observer was
  // ever attached. Infinite scroll was then dead for the rest of the session.
  // A callback ref fires exactly when the node mounts or unmounts, so no
  // dependency list can drift out of sync with the DOM again.
  const observerRef = useRef<IntersectionObserver | null>(null);
  // Read through refs so the observer never needs rebuilding when these change.
  const loadNextPageRef = useRef(loadNextPage);
  const loadMoreErrorRef = useRef(loadMoreError);
  useEffect(() => {
    loadNextPageRef.current = loadNextPage;
    loadMoreErrorRef.current = loadMoreError;
  }, [loadNextPage, loadMoreError]);

  const attachSentinel = useCallback((node: HTMLDivElement | null) => {
    observerRef.current?.disconnect();
    observerRef.current = null;
    if (!node) return;
    const observer = new IntersectionObserver(
      (entries) => {
        // `loadNextPage` already guards hasMore/searchLoading/loadMoreLoading.
        if (entries[0]?.isIntersecting && !loadMoreErrorRef.current) {
          loadNextPageRef.current();
        }
      },
      { rootMargin: '600px' },
    );
    observer.observe(node);
    observerRef.current = observer;
  }, []);

  useEffect(() => () => observerRef.current?.disconnect(), []);


  // ---- Render ----
  // The bulk-install dialogs (instance picker + canonical InstallFlow) are
  // rendered by BOTH the standard page AND the Bazaar page — the Bazaar's
  // "N picked" button drives the exact same reviewed pipeline, and it must
  // open on the Bazaar rather than bouncing to the normal Browse view.
  const bulkInstallDialogs = (
    <>
      {/* Instance picker for bulk install when no instance context is set */}
      <Dialog open={pickerOpen} onOpenChange={(open) => { if (!open) setPickerOpen(false); }}>
        <DialogContent className="max-w-md">
          <DialogTitle>
            Install {selectedItems.size} selected item{selectedItems.size === 1 ? '' : 's'}
          </DialogTitle>
          <DialogDescription>
            Choose an instance. The latest compatible version of each item will be installed.
          </DialogDescription>
          {instances.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No instances yet. Create an instance from My Instances first.
            </p>
          ) : (
            <>
              <select
                value={pickerInstanceId}
                onChange={(e) => setPickerInstanceId(e.target.value)}
                aria-label="Target instance"
                className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
              >
                <option value="">Choose an instance…</option>
                {instances.map((instance) => (
                  <option key={instance.instance_id} value={instance.instance_id}>
                    {instance.name} — {instance.loader} · MC {instance.minecraft_version}
                  </option>
                ))}
              </select>
              <div className="flex justify-end gap-2">
                <button
                  type="button"
                  onClick={() => setPickerOpen(false)}
                  className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent"
                >
                  Cancel
                </button>
                <button
                  type="button"
                  onClick={confirmPickerInstall}
                  disabled={!pickerInstanceId}
                  className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
                >
                  Install {selectedItems.size} item{selectedItems.size === 1 ? '' : 's'}
                </button>
              </div>
            </>
          )}
        </DialogContent>
      </Dialog>

      {/* Canonical install pipeline for the selected batch */}
      {installTarget && (
        <InstallFlow
          open
          intent={installTarget.intent}
          instanceName={installTarget.instanceName}
          initialPlan={installTarget.initialPlan}
          initialError={installTarget.initialError}
          background
          autoBackground={autoConfirmCleanInstalls}
          alwaysAutoConfirm={alwaysAutoConfirmInstalls}
          onBackgroundStart={(plan) => {
            const count = plan.intent.action.type === 'batch-install'
              ? plan.intent.action.items.length
              : 1;
            clearSelection();
            startPlan(
              plan,
              bulkInstallLabel(count),
              installTarget.instanceName,
              (error, failedPlan) => {
                bulkInstallBusyRef.current = false;
                setInstallTarget({
                  intent: installTarget.intent,
                  instanceName: installTarget.instanceName,
                  initialPlan: failedPlan,
                  initialError: {
                    message: error,
                    code: 'ERR_APPLY',
                    skipItemId: failedBatchRootItemId(failedPlan, error),
                  },
                });
              },
            );
          }}
          onOpenInstance={onOpenInstance}
          onClose={() => {
            // Keep the selection when the review is cancelled so the user can
            // adjust it and try again; background starts clear it themselves.
            setInstallTarget(null);
            bulkInstallBusyRef.current = false;
          }}
        />
      )}
    </>
  );

  // High Interaction Browse (the Bazaar) is its own page: the standard search
  // chrome stays off-screen so the playful presentation owns the whole view.
  if (bazaarMode) {
    return (
      <div className="space-y-6" data-tour="page-browse">
        <BrowseBazaar
          items={bazaarItems}
          instanceVersion={bazaarInstanceVersion}
          ownedIds={bazaarOwnedIds}
          onAdd={(item) => handleSelectItem(item.id)}
          onOpenMod={(item) => handleSelectItem(item.id)}
          onExit={() => setBazaarMode(false)}
          hasMore={hasMore && !searchLoading}
          loadMoreLoading={loadMoreLoading}
          onLoadMore={loadNextPage}
          onInstallBag={handleBazaarInstall}
          stall={bazaarStall}
          onStallChange={setBazaarStall}
          initialSettledOrder={bazaarSettledOrderRef.current}
          onSettledOrderChange={(order) => { bazaarSettledOrderRef.current = order; }}
        />
        {bulkInstallDialogs}
      </div>
    );
  }

  return (
    <div className="space-y-6" data-tour="page-browse">
      <section className="agora-hero compact">
        <div className="flex items-center gap-3 mb-2">
          <h2 className="text-2xl font-bold">Browse</h2>
          {/* Simple mode has exactly one Browse. Offering a door to the Bazaar
              from it would put the thing the mode exists to avoid one click
              away — the interaction mode is changed in Settings, not here. */}
          {simpleBrowse ? null : (
            <button
              type="button"
              onClick={() => setBazaarMode(true)}
              className="ml-auto rounded-lg border border-primary bg-primary px-4 py-2 text-sm font-bold text-primary-foreground shadow-sm hover:bg-primary/90"
              data-testid="open-bazaar"
            >
              The Bazaar 🎡
            </button>
          )}
          {/* <span className="rounded-full bg-muted text-muted-foreground px-2 py-0.5 text-xs font-medium uppercase tracking-wide">
            Preview
          </span> */}
        </div>
        <p className="text-muted-foreground">
          Curated mods, packs, shaders, resource packs, and more.
          {simpleBrowse ? '' : ' High Interaction fans can wander The Bazaar instead.'}
        </p>
      </section>

      {/* Registry offline banner */}
      <RegistryStatusView
        variant="banner"
        state={regState}
        status={regStatus}
        error={regError}
        actions={regActions}
      />

      <section className="rounded-xl border border-border bg-card p-4">
        <label className="block text-sm font-semibold" htmlFor="browse-instance-context">
          Discover for an instance
        </label>
        <p className="mt-1 text-xs text-muted-foreground">
          Select an instance to apply its exact Minecraft version and loader and see installed/update labels.
        </p>
        <select
          id="browse-instance-context"
          value={activeInstanceId}
          onChange={(event) => { void selectInstanceContext(event.target.value); }}
          disabled={contextLoading}
          className="mt-3 w-full rounded-lg border border-input bg-background px-3 py-2 text-sm disabled:opacity-50"
        >
          <option value="">No instance context</option>
          {instances.map((instance) => (
            <option key={instance.instance_id} value={instance.instance_id}>
              {instance.name} — {instance.loader} · MC {instance.minecraft_version}
            </option>
          ))}
        </select>
        {contextLoading && <p className="mt-2 text-xs text-muted-foreground">Checking compatibility and updates…</p>}
        {contextError && <p className="mt-2 text-xs text-destructive">{contextError}</p>}
      </section>

      {/* Community-registry pitch: worth reading once, but it is an appeal, not
          a control, and Simple mode keeps the page to the things that find a
          mod. It stays reachable from the About and Governance pages. */}
      {simpleBrowse ? null : (
      <section className="rounded-xl border border-border bg-card p-4">
        <div className="flex items-start gap-3">
          <Leaf aria-hidden className="mt-0.5 h-5 w-5 text-sea-blue" />
          <div className="flex-1">
            <p className="text-sm font-semibold text-foreground">
              Help grow the community registry
            </p>
            <p className="text-xs text-muted-foreground mt-1">
              Agora's curated catalog is built by the community, for the community.
              We're assembling links to our favorite mods — ideally through
              alternative hosting options like GitHub Releases rather than
              centralized platforms. Every contribution counts.
            </p>
            {agoraRepositoryUrl && (
              <a
                href={agoraRepositoryUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="mt-2 inline-block text-xs font-medium text-primary hover:underline"
              >
                Contribute on GitHub ↗
              </a>
            )}
          </div>
        </div>
      </section>
      )}

      <div className="flex flex-col lg:flex-row lg:flex-wrap gap-4">
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search mods, packs, and more…"
          className="flex-1 rounded-lg border border-input bg-background px-4 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
        />
        <div className="flex items-center gap-1 rounded-lg border border-input bg-background p-0.5">
          <button
            onClick={() => {
              setLayout('list');
              try { localStorage.setItem('browse_layout', 'list'); } catch { /* ignore */ }
            }}
            className={`rounded-md p-1.5 transition-colors ${layout === 'list' ? 'bg-accent text-accent-foreground' : 'text-muted-foreground hover:text-foreground'}`}
            title="List view"
          >
            <List size={18} />
          </button>
          <button
            onClick={() => {
              setLayout('grid');
              try { localStorage.setItem('browse_layout', 'grid'); } catch { /* ignore */ }
            }}
            className={`rounded-md p-1.5 transition-colors ${layout === 'grid' ? 'bg-accent text-accent-foreground' : 'text-muted-foreground hover:text-foreground'}`}
            title="Grid view"
          >
            <LayoutGrid size={18} />
          </button>
        </div>
        <select
          aria-label="Content type"
          value={contentType ?? ''}
          onChange={(e) => setContentType(e.target.value || null)}
          disabled={sort === 'for_you'}
          className="rounded-lg border border-input bg-background px-3 py-2 text-sm disabled:opacity-40 disabled:cursor-not-allowed"
        >
          <option value="">All types</option>
          {CONTENT_TYPES.map((ct) => (
            <option key={ct} value={ct}>{ct}</option>
          ))}
        </select>
        <select
          value={mcVersion ?? ''}
          onChange={(e) => setMcVersion(e.target.value || null)}
          className="rounded-lg border border-input bg-background px-3 py-2 text-sm"
          title="Filter by Minecraft version"
        >
          <option value="">Any MC version</option>
          {mcVersions.map((v) => (
            <option key={v} value={v}>MC {v}</option>
          ))}
        </select>
        <select
          value={loader ?? ''}
          onChange={(e) => setLoader(e.target.value || null)}
          className="rounded-lg border border-input bg-background px-3 py-2 text-sm"
          title="Filter by modloader"
        >
          <option value="">Any loader</option>
          {loaders.map((l) => (
            <option key={l} value={l}>{l}</option>
          ))}
        </select>
        <select
          value={sort}
          onChange={(e) => handleSortChange(e.target.value as SortOption)}
          className="rounded-lg border border-input bg-background px-3 py-2 text-sm"
        >
          {sortOptions.map((s) => (
            <option key={s.value} value={s.value}>{s.label}</option>
          ))}
        </select>
      </div>

      {/* Active filter chips */}
      {(mcVersion || loader) && (
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <span>Active filters:</span>
          {mcVersion && (
            <button
              onClick={() => setMcVersion(null)}
              className="rounded-full border border-border px-2 py-0.5 hover:bg-accent"
            >
              MC {mcVersion} ✕
            </button>
          )}
          {loader && (
            <button
              onClick={() => setLoader(null)}
              className="rounded-full border border-border px-2 py-0.5 hover:bg-accent"
            >
              {loader} ✕
            </button>
          )}
        </div>
      )}

      {/* Category filters */}
      {sort !== 'for_you' && (
        <div className="space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <p className="text-sm font-semibold">Modrinth categories</p>
              <p className="text-xs text-muted-foreground">
                {contentType ? `Categories for ${contentType} content.` : 'Combined categories for all content types.'}
              </p>
            </div>
            <CuratedCategoryDropdown
              categories={visibleCuratedCategories}
              selectedCategory={category}
              onSelect={setCategory}
              disabled={metaLoading || visibleCuratedCategories.length === 0}
            />
          </div>
          {/* Simple mode keeps category filtering but not the wall of it: the
              dropdown above picks the same categories from one control instead
              of dozens of chips competing with the results underneath. */}
          {simpleBrowse ? null : (
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => setCategory(null)}
              className={[
                'px-3 py-1 rounded-full text-sm border transition-colors',
                category === null
                  ? 'bg-primary text-primary-foreground border-primary'
                  : 'border-border hover:bg-accent',
              ].join(' ')}
            >
              All
            </button>
            {visibleModrinthCategories.map((item) => (
              <button
                type="button"
                key={item.name}
                onClick={() => setCategory(item.name)}
                className={[
                  'px-3 py-1 rounded-full text-sm border transition-colors',
                  category === item.name
                    ? 'bg-primary text-primary-foreground border-primary'
                    : 'border-border hover:bg-accent',
                ].join(' ')}
              >
                {formatCategoryName(item.name)}
              </button>
            ))}
            {!metaLoading && visibleModrinthCategories.length === 0 && (
              <span className="self-center text-xs text-muted-foreground">
                No Modrinth categories are available for this content type.
              </span>
            )}
          </div>
          )}
        </div>
      )}

      {/* Metadata error (categories) — separate from search error */}
      {metaError && (
        <div className="rounded-lg border border-destructive bg-destructive/10 p-3 text-xs text-destructive">
          Could not load categories: {metaError}
        </div>
      )}

      {/* Search error with Retry and Clear Filters */}
      {searchError && (
        <div className="rounded-lg border border-destructive bg-destructive/10 p-3 text-sm text-destructive">
          <p>{searchError}</p>
          <div className="mt-2 flex gap-2">
            <button
              onClick={() => {
                setSearchError(null);
                setSearchLoading(true);
                // Bump generation to re-trigger the search effect
                generationRef.current += 1;
                const newGen = generationRef.current;
                inFlightSearchRef.current = newGen;
                setItems([]);
                setHasMore(false);

                // Re-run the current query
                const run = async () => {
                  try {
                    if (sort === 'for_you') {
                      // simplified for_you retry
                      const registryItems = await forYouItems(
                        mcVersion ?? undefined,
                        loader ?? undefined,
                        undefined,
                        undefined,
                        debouncedQuery.trim() || undefined,
                      );
                      if (inFlightSearchRef.current === newGen) {
                        setItems(curatedOnlyItems(registryItems));
                        setHasMore(false);
                      }
                    } else {
                      const page = await browseSearch(
                        queryKey,
                        debouncedQuery.trim() || undefined,
                        contentType ?? undefined,
                        category ?? undefined,
                        sort,
                        mcVersion ?? undefined,
                        loader ?? undefined,
                      );
                      if (inFlightSearchRef.current === newGen) {
                        setItems(page.items);
                        setHasMore(page.hasMore);
                      }
                    }
                  } catch (e) {
                    if (inFlightSearchRef.current === newGen) {
                      setSearchError(formatError(e));
                    }
                  } finally {
                    if (inFlightSearchRef.current === newGen) {
                      setSearchLoading(false);
                      inFlightSearchRef.current = null;
                    }
                  }
                };
                run();
              }}
              className="rounded-lg bg-primary px-3 py-1 text-xs font-medium text-primary-foreground hover:bg-primary/90"
            >
              Retry
            </button>
            {hasActiveFilters && (
              <button
                onClick={clearFilters}
                className="rounded-lg border border-border px-3 py-1 text-xs font-medium text-muted-foreground hover:bg-accent"
              >
                Clear Filters
              </button>
            )}
          </div>
        </div>
      )}

      {/* Loading or results */}
      {searchLoading ? (
        <div className="rounded-xl border border-dashed border-border bg-card p-6 text-center text-muted-foreground">
          Loading items…
        </div>
      ) : items.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border bg-card p-6 text-center">
          <p className="text-muted-foreground">No items to display.</p>
          {hasActiveFilters && (
            <button
              onClick={clearFilters}
              className="mt-2 rounded-lg bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90"
            >
              Clear Filters
            </button>
          )}
        </div>
      ) : layout === 'grid' ? (
        <div data-tour="browse-results">
          <BrowseTileResults
            items={items}
            contextFor={contextFor}
            onSelectMod={handleSelectItem}
            selectedItems={selectedItems}
            onToggleSelect={handleToggleSelect}
          />
        </div>
      ) : (
        <div data-tour="browse-results">
          <BrowseListResults
            items={items}
            contextFor={contextFor}
            onSelectMod={handleSelectItem}
            selectedItems={selectedItems}
            onToggleSelect={handleToggleSelect}
          />
        </div>
      )}

      {/* Infinite scroll sentinel */}
      {loadMoreError && (
        <div className="rounded-lg border border-destructive bg-destructive/10 p-3 text-sm text-destructive">
          <p>{loadMoreError}</p>
          <button
            onClick={loadNextPage}
            className="mt-2 rounded-lg bg-primary px-3 py-1 text-xs font-medium text-primary-foreground hover:bg-primary/90"
          >
            Retry loading more
          </button>
        </div>
      )}
      {hasMore && !searchLoading && (
        <div
          ref={attachSentinel}
          data-testid="browse-load-sentinel"
          className="py-6 text-center text-sm text-muted-foreground"
        >
          {loadMoreLoading ? 'Loading more…' : ''}
        </div>
      )}
      {!hasMore && items.length > 0 && (
        <p className="py-4 text-center text-xs text-muted-foreground">All results loaded</p>
      )}

      {/* Bulk-selection install bar */}
      {selectedItems.size > 0 && (
        <div className="pointer-events-none fixed inset-x-0 bottom-5 z-50 flex justify-center px-4">
          <div className="pointer-events-auto flex flex-wrap items-center justify-center gap-2 rounded-full border border-border bg-card px-4 py-2 shadow-xl">
            <span className="text-sm font-medium" data-testid="browse-selection-count">
              {selectedItems.size} selected
            </span>
            <button
              type="button"
              onClick={handleInstallSelected}
              className="inline-flex items-center gap-1.5 rounded-full bg-primary px-4 py-1.5 text-sm font-semibold text-primary-foreground hover:bg-primary/90"
            >
              <Download aria-hidden size={14} />
              Install {selectedItems.size}
              {installBarTargetName ? ` to ${installBarTargetName}` : ''}
            </button>
            <button
              type="button"
              onClick={() => setSelectedItems(new Map())}
              className="rounded-full border border-border px-3 py-1.5 text-sm font-medium text-muted-foreground hover:bg-accent"
            >
              Clear
            </button>
          </div>
        </div>
      )}

      {/* Bulk-install picker + InstallFlow (shared with the Bazaar page). */}
      {bulkInstallDialogs}
    </div>
  );
}
