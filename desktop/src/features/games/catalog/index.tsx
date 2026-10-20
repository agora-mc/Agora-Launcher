/**
 * Catalog slot owned by the catalog slice: the Library tab's curated catalog section.
 */

import { SlotPlaceholder } from '../shared/SlotPlaceholder';

export function LibraryCatalogSection({ gameId }: { gameId: string }) {
  return <SlotPlaceholder title={`Curated catalog (${gameId})`} slice="catalog" />;
}
