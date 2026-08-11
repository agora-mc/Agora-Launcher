// Negative fixture: the live READ layer may only invoke read-only Tauri
// commands. A settings mutation must fail the boundary.
import { setSetting } from '@/lib/tauri';

export function fixtureLiveSettings() {
  return setSetting;
}
