// Negative fixture (SOL-2 BLOCKER C): a DYNAMIC import of the app layer cannot
// have its command names verified — it must NEVER be treated as type-only.
export async function fixtureLiveDynamic() {
  const tauri = await import('@/lib/tauri');
  return tauri.createInstance;
}
