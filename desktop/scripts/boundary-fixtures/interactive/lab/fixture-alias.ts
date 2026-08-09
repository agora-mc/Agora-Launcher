// Negative fixture: app-layer alias import must be rejected.
import { someCommand } from '@/lib/tauri';

export function fixtureAlias() {
  return someCommand;
}
