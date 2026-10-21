/**
 * Tools slot owned by the tools slice: an instance's Tools tab.
 */

import { SlotPlaceholder } from '../shared/SlotPlaceholder';

export function InstanceToolsTab({ gameId, instanceId }: { gameId: string; instanceId: string }) {
  return <SlotPlaceholder title={`Tools: ${instanceId} (${gameId})`} slice="tools" />;
}
