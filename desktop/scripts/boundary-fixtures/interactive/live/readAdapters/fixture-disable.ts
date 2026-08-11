// Negative fixture: the live READ layer may only invoke read-only Tauri
// commands. A disable mutation must fail the boundary.
import { disableInstanceMod } from '@/lib/tauri';

export function fixtureLiveDisable() {
  return disableInstanceMod;
}
