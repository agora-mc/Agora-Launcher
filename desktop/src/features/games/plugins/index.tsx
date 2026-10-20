/**
 * Plugins slot owned by the plugins slice: an instance's Plugins tab.
 */

import { SlotPlaceholder } from '../shared/SlotPlaceholder';

export function InstancePluginsTab({ gameId, instanceId }: { gameId: string; instanceId: string }) {
  return <SlotPlaceholder title={`Plugins: ${instanceId} (${gameId})`} slice="plugins" />;
}
