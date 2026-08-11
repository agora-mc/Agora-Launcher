// Negative fixture: the live READ layer may only invoke read-only Tauri
// commands. An install mutation must fail the boundary.
import { createInstance } from '@/lib/tauri';

export function fixtureLiveInstall() {
  return createInstance;
}
