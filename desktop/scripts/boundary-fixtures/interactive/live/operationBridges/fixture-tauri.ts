// Negative fixture: operation bridges host approved Standard controllers only
// and must never invoke Tauri commands directly.
import { disableInstanceMod } from '@/lib/tauri';

export function fixtureBridgeTauri() {
  return disableInstanceMod;
}
