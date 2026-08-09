// Negative fixture: a Tauri package is never an allowed external dependency
// for shared visual code.
import { invoke } from '@tauri-apps/api/core';

export function fixtureTauri() {
  return invoke;
}
