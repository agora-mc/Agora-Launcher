/**
 * Bases slot owned by the bases slice: the Setup tab's pinned bases (MASTER_SPEC §26.4).
 */

import { BasesSection } from './BasesSection';

export function SetupBasesSection({ gameId }: { gameId: string }) {
  return <BasesSection gameId={gameId} />;
}
