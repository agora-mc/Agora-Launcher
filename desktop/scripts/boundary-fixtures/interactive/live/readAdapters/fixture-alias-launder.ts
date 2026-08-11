// Negative fixture (SOL-2 BLOCKER C): an ALIASED mutation import must be judged
// by its ORIGINAL name, not its local binding. `restoreSnapshot as
// getInstanceDetail` must be flagged as a non-read command.
import { restoreSnapshot as getInstanceDetail } from '@/lib/tauri';

export function fixtureAliasLaunder() {
  return getInstanceDetail;
}
