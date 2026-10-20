/**
 * Instances slots owned by the instances slice: the game's instance list (the Instances tab)
 * and the Overview tab of an instance's page.
 */

import { SlotPlaceholder } from '../shared/SlotPlaceholder';

interface GameInstancesPageProps {
  gameId: string;
  /** Open an instance's page. The shell owns the navigation; the slice only calls this. */
  onOpenInstance: (instanceId: string) => void;
}

export function GameInstancesPage({ gameId }: GameInstancesPageProps) {
  return <SlotPlaceholder title={`Instances (${gameId})`} slice="instances" />;
}

export function InstanceOverviewTab({ gameId, instanceId }: { gameId: string; instanceId: string }) {
  return <SlotPlaceholder title={`Overview: ${instanceId} (${gameId})`} slice="instances" />;
}
