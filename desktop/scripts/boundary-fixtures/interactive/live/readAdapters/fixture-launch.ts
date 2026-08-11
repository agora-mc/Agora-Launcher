// Negative fixture: the live READ layer may only invoke read-only Tauri
// commands. A launch mutation must fail the boundary.
import { launchInstance } from '@/lib/tauri';

export function fixtureLiveLaunch() {
  return launchInstance;
}
