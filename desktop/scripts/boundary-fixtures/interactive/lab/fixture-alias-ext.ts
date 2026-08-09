// Negative fixture: alias import with an explicit extension must be rejected.
import { someCommand } from '@/lib/tauri.ts';

export function fixtureAliasExt() {
  return someCommand;
}
