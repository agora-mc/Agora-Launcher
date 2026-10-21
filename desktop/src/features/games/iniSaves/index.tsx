/**
 * INI and saves slot owned by the iniSaves slice: an instance's "INI & saves" tab.
 */

import { SlotPlaceholder } from '../shared/SlotPlaceholder';

export function InstanceIniSavesTab({ gameId, instanceId }: { gameId: string; instanceId: string }) {
  return <SlotPlaceholder title={`INI & saves: ${instanceId} (${gameId})`} slice="iniSaves" />;
}
