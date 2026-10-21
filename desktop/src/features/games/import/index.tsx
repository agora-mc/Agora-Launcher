/**
 * Import slot owned by the import slice: the Import tab (Mod Organizer 2 setups).
 */

import { SlotPlaceholder } from '../shared/SlotPlaceholder';

export function GameImportPage({ gameId }: { gameId: string }) {
  return <SlotPlaceholder title={`Import (${gameId})`} slice="import" />;
}
