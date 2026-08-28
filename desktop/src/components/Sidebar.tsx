import { useRef } from 'react';
import type { LucideIcon } from 'lucide-react';
import { ChevronLeft, ChevronRight, Command } from 'lucide-react';
import { BrandMark } from './BrandMark';
import type { Tab } from '../lib/useDestination';
import type { RegistryStatus } from '../lib/tauri';

interface SidebarProps {
  tabs: { id: Tab; label: string; icon: LucideIcon }[];
  activeTab: Tab;
  onSelectTab: (tab: Tab) => void;
  onOpenCommandPalette?: () => void;
  collapsed: boolean;
  width: number;
  onCollapsedChange: (collapsed: boolean) => void;
  onWidthChange: (width: number) => void;
  onWidthCommit: (width: number) => void;
  registryStatus?: RegistryStatus | null;
}

const MIN_WIDTH = 180;
const MAX_WIDTH = 420;
const DEFAULT_WIDTH = 256;

/**
 * Guided-tour anchors for the nav items the walkthrough points at
 * (`features/tour`). Spelled out rather than derived from the tab id so the
 * anchor audit in `tourAnchors.test.ts` can see every one of them.
 */
const TOUR_ANCHORS: Partial<Record<Tab, string>> = {
  home: 'nav-home',
  browse: 'nav-browse',
  instances: 'nav-instances',
  governance: 'nav-governance',
  guide: 'nav-guide',
  'field-guide': 'nav-field-guide',
  about: 'nav-about',
  settings: 'nav-settings',
};

function clampWidth(width: number) {
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, width));
}

export function Sidebar({
  tabs,
  activeTab,
  onSelectTab,
  onOpenCommandPalette,
  collapsed,
  width,
  onCollapsedChange,
  onWidthChange,
  onWidthCommit,
  registryStatus,
}: SidebarProps) {
  const latestWidth = useRef(width);

  const startResize = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = width;
    latestWidth.current = width;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';

    const move = (moveEvent: PointerEvent) => {
      const nextWidth = clampWidth(startWidth + moveEvent.clientX - startX);
      latestWidth.current = nextWidth;
      onWidthChange(nextWidth);
    };
    const finish = () => {
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', finish);
      onWidthCommit(latestWidth.current);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', finish, { once: true });
  };

  const resizeWithKeyboard = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    const amount = event.shiftKey ? 32 : 8;
    const nextWidth = clampWidth(width + (event.key === 'ArrowRight' ? amount : -amount));
    onWidthChange(nextWidth);
    onWidthCommit(nextWidth);
  };

  return (
    // The two controls that overhang the sidebar's right edge — the collapse
    // arrow and the resize grip — are SIBLINGS of the <aside>, not children.
    // The aside is backdrop-blurred, and a child that spills past a
    // backdrop-filtered element is rasterised through that filter: the arrow
    // came out visibly soft while everything inside the sidebar stayed sharp.
    // Out here they render crisp, and the wrapper carries the same box, so the
    // positioning is unchanged.
    <div className="relative flex shrink-0" style={{ width: collapsed ? 64 : width }}>
      <aside
        className="app-sidebar decorative-shell flex h-full w-full min-w-0 flex-col overflow-hidden rounded-xl border border-border bg-card/95 shadow-[4px_0_24px_hsl(var(--midnight)/0.04)] backdrop-blur"
        data-testid="sidebar"
      >
      <div className={`border-b border-border ${collapsed ? 'p-3' : 'p-4'}`}>
        <BrandMark compact={collapsed} className={collapsed ? 'justify-center' : ''} />
      </div>

      <nav className="min-h-0 flex-1 space-y-1 overflow-y-auto p-3" aria-label="Main navigation">
        {tabs.map((tab) => {
          const isActive = activeTab === tab.id;
          const Icon = tab.icon;
          return (
            <button
              key={tab.id}
              onClick={() => onSelectTab(tab.id)}
              aria-current={isActive ? 'page' : undefined}
              title={collapsed ? tab.label : undefined}
              data-tour={TOUR_ANCHORS[tab.id]}
              className={[
                'flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left text-sm font-medium transition-colors',
                collapsed ? 'px-0 justify-center' : '',
                isActive
                  ? 'bg-primary text-primary-foreground shadow-sm'
                  : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
              ].join(' ')}
            >
              <Icon className="h-[18px] w-[18px] shrink-0" aria-hidden="true" />
              {!collapsed && <span className="min-w-0 flex-1 text-left leading-snug">{tab.label}</span>}
            </button>
          );
        })}
      </nav>

      {!collapsed && (
        <div className="space-y-1 border-t border-border p-3">
          <button
            onClick={onOpenCommandPalette}
            className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            aria-label="Open command palette"
          >
            <Command className="h-4 w-4" aria-hidden="true" />
            <span className="min-w-0 flex-1 text-left leading-snug">Quick actions</span>
            <kbd className="ml-auto rounded border border-border bg-background px-1.5 py-0.5 font-mono text-[10px]">Ctrl K</kbd>
          </button>
        </div>
      )}

      {!collapsed && (
        <div className="border-t border-border p-4 text-xs text-muted-foreground">
          <div>v{__APP_VERSION__} · Community curated</div>
          <div className="mt-1">
            {registryStatus == null ? (
              'Checking registry…'
            ) : registryStatus.has_cached_db ? (
              registryStatus.cached_tag
                ? `Registry ${registryStatus.cached_tag}`
                : `Local registry · schema v${registryStatus.cached_schema_version ?? '—'}`
            ) : (
              'No registry loaded'
            )}
          </div>
        </div>
      )}
      </aside>

      <button
        onClick={() => onCollapsedChange(!collapsed)}
        className="absolute -right-3 top-20 z-10 flex h-6 w-6 items-center justify-center rounded-full border border-border bg-card text-muted-foreground shadow-sm hover:bg-accent"
        aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
      >
        {collapsed ? <ChevronRight className="h-3.5 w-3.5" /> : <ChevronLeft className="h-3.5 w-3.5" />}
      </button>

      {!collapsed && (
        <div
          role="separator"
          aria-label="Resize sidebar"
          aria-orientation="vertical"
          aria-valuemin={MIN_WIDTH}
          aria-valuemax={MAX_WIDTH}
          aria-valuenow={Math.round(width)}
          tabIndex={0}
          onPointerDown={startResize}
          onKeyDown={resizeWithKeyboard}
          onDoubleClick={() => {
            onWidthChange(DEFAULT_WIDTH);
            onWidthCommit(DEFAULT_WIDTH);
          }}
          className="absolute inset-y-0 -right-1 z-[5] w-2 cursor-col-resize touch-none focus:outline-none focus:ring-2 focus:ring-inset focus:ring-ring"
        />
      )}
    </div>
  );
}
