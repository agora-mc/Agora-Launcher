/**
 * FOMOD slot owned by the fomod slice: the installer wizard for archives that ask questions.
 * It has no shell slot of its own; the content slice mounts it from the instance's Content tab.
 */

import { SlotPlaceholder } from '../shared/SlotPlaceholder';

export function FomodWizard({ contentId }: { contentId: string }) {
  return <SlotPlaceholder title={`Installer options (${contentId})`} slice="fomod" />;
}
