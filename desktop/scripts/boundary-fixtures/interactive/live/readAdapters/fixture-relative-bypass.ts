// Negative fixture (SOL-2 BLOCKER C): a RELATIVE path must not smuggle the
// app layer past the `@/lib/tauri` named-import check. This resolves outside
// the interactive root into the app layer and must fail the boundary.
import { createInstance } from '../../../../../src/lib/tauri';

export function fixtureLiveRelativeBypass() {
  return createInstance;
}
