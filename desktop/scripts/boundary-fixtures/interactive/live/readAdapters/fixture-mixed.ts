// Negative fixture (SOL-2 BLOCKER C): a PROHIBITED value import followed by a
// SAFE value import from the same specifier must still be flagged. The
// analyzer aggregates every matching form — the later allowlisted import must
// never hide the earlier mutation.
import { restoreSnapshot } from '@/lib/tauri';
import { getInstanceDetail } from '@/lib/tauri';

export function fixtureMixed() {
  return getInstanceDetail;
}
