// Negative fixture: dynamic import of the app Tauri wrapper must be rejected.
export async function fixtureDynamic() {
  const module = await import('@/lib/tauri');
  return module;
}
