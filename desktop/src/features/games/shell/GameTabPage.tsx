/**
 * The page for one of a non-Minecraft game's four tabs. The shell decides which slots a tab
 * has; each slot's content comes from its owning slice's folder.
 */

import type { GameTabId } from '@/lib/useDestination';
import { GameInstancesPage } from '../instances';
import { LibraryStoreSection } from '../content';
import { LibraryCatalogSection } from '../catalog';
import { GameImportPage } from '../import';
import { SetupInstallsSection } from '../discovery';
import { SetupBasesSection } from '../bases';
import { SetupRecoverySection } from '../userFiles';

interface GameTabPageProps {
  gameId: string;
  tab: GameTabId;
  /** Open an instance's page from the Instances tab. */
  onOpenInstance: (instanceId: string) => void;
}

export function GameTabPage({ gameId, tab, onOpenInstance }: GameTabPageProps) {
  switch (tab) {
    case 'instances':
      return <GameInstancesPage gameId={gameId} onOpenInstance={onOpenInstance} />;
    case 'library':
      return (
        <div className="space-y-6">
          <LibraryStoreSection gameId={gameId} />
          <LibraryCatalogSection gameId={gameId} />
        </div>
      );
    case 'import':
      return <GameImportPage gameId={gameId} />;
    case 'setup':
      return (
        <div className="space-y-6">
          <SetupInstallsSection gameId={gameId} />
          <SetupBasesSection gameId={gameId} />
          <SetupRecoverySection gameId={gameId} />
        </div>
      );
  }
}
