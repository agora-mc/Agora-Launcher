/**
 * Content slots owned by the content slice: the Library tab's store section and an
 * instance's Content tab (which also hosts the FOMOD wizard).
 */

import { SlotPlaceholder } from '../shared/SlotPlaceholder';

export function LibraryStoreSection({ gameId }: { gameId: string }) {
  return <SlotPlaceholder title={`Content store (${gameId})`} slice="content" />;
}

export function InstanceContentTab({ gameId, instanceId }: { gameId: string; instanceId: string }) {
  return <SlotPlaceholder title={`Content: ${instanceId} (${gameId})`} slice="content" />;
}
