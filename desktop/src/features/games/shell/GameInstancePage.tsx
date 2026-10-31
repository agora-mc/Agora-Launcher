/**
 * An instance of a non-Minecraft game, with its five pages (Overview, Content, Plugins,
 * INI & saves, Tools). The shell draws the header and the page switch; each page is a slot.
 */

import type { GameInstancePage as GameInstancePageId } from '@/lib/useDestination';
import { InstanceOverviewTab } from '../instances';
import { InstanceContentTab } from '../content';
import { InstancePluginsTab } from '../plugins';
import { InstanceIniSavesTab } from '../iniSaves';
import { InstanceToolsTab } from '../tools';
import { GAME_INSTANCE_PAGE_TABS } from './gameNav';

interface GameInstancePageProps {
  gameId: string;
  instanceId: string;
  page: GameInstancePageId;
  onNavigatePage: (page: GameInstancePageId) => void;
  onBack: () => void;
}

export function GameInstancePage({ gameId, instanceId, page, onNavigatePage, onBack }: GameInstancePageProps) {
  return (
    <div className="space-y-6" data-testid="game-instance-page">
      <button
        type="button"
        onClick={onBack}
        className="rounded-lg border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent"
      >
        ← Instances
      </button>
      <nav aria-label="Instance pages" className="flex flex-wrap gap-2">
        {GAME_INSTANCE_PAGE_TABS.map((entry) => {
          const isActive = entry.id === page;
          return (
            <button
              key={entry.id}
              type="button"
              aria-current={isActive ? 'page' : undefined}
              onClick={() => onNavigatePage(entry.id)}
              className={[
                'rounded-lg px-3 py-1.5 text-sm font-medium transition-colors',
                isActive
                  ? 'bg-primary text-primary-foreground shadow-sm'
                  : 'border border-border text-muted-foreground hover:bg-accent hover:text-accent-foreground',
              ].join(' ')}
            >
              {entry.label}
            </button>
          );
        })}
      </nav>
      {page === 'overview' && <InstanceOverviewTab gameId={gameId} instanceId={instanceId} />}
      {page === 'content' && <InstanceContentTab gameId={gameId} instanceId={instanceId} />}
      {page === 'plugins' && <InstancePluginsTab gameId={gameId} instanceId={instanceId} />}
      {page === 'ini-saves' && <InstanceIniSavesTab gameId={gameId} instanceId={instanceId} />}
      {page === 'tools' && <InstanceToolsTab gameId={gameId} instanceId={instanceId} />}
    </div>
  );
}
