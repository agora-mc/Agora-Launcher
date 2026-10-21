/**
 * Discovery slot owned by the discovery slice: the Setup tab's installs and custom folders.
 */

import { SlotPlaceholder } from '../shared/SlotPlaceholder';

export function SetupInstallsSection({ gameId }: { gameId: string }) {
  return <SlotPlaceholder title={`Installs and folders (${gameId})`} slice="discovery" />;
}
