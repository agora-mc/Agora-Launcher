// Negative fixture (SOL-2 BLOCKER C): a RE-EXPORT of a mutation command must
// be inspected like a static import (not silently treated as type-only).
export { createInstance } from '@/lib/tauri';
