// Negative fixture: a sibling folder that shares the "interactive" string
// prefix (`interactive-app`) must NOT be accepted as an internal interactive
// edge. Path containment is segment-based, so this resolves outside the scan
// root and must be rejected.
import { controller } from '../../interactive-app/lab/controller';

export function fixturePrefixCollision() {
  return controller;
}
