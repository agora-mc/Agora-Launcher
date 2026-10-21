/**
 * User-files slot owned by the user-files slice: the Setup tab's recovery section.
 */

import { SlotPlaceholder } from '../shared/SlotPlaceholder';

export function SetupRecoverySection({ gameId }: { gameId: string }) {
  return <SlotPlaceholder title={`Recovery (${gameId})`} slice="userFiles" />;
}
