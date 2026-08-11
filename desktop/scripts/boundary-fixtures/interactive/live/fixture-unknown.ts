// Negative fixture: a live/ file that is not in a known subarea (read, core,
// or operationBridges) must fail the boundary.
import { getInstanceDetail } from '@/lib/tauri';

export function fixtureUnknownLive() {
  return getInstanceDetail;
}
