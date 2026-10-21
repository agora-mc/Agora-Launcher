/**
 * Bases slot owned by the bases slice: the Setup tab's pinned bases.
 */

import { SlotPlaceholder } from '../shared/SlotPlaceholder';

export function SetupBasesSection({ gameId }: { gameId: string }) {
  return <SlotPlaceholder title={`Pinned bases (${gameId})`} slice="bases" />;
}
