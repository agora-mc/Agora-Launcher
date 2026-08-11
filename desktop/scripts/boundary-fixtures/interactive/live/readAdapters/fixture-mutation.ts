// Negative fixture: the live READ layer may only invoke read-only Tauri
// commands. A mutation import (restore) must fail the boundary.
import { restoreSnapshot } from '@/lib/tauri';

export function fixtureLiveMutation() {
  return restoreSnapshot;
}
