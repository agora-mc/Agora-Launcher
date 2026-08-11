// Negative fixture: an unclassified source directly under the interactive scan
// root must be rejected (only live/ is exempt from classification).
import { someCommand } from '@/lib/tauri';

export function fixtureUnclassified() {
  return someCommand;
}
