/**
 * NEGATIVE FIXTURE — must violate the ambience boundary.
 * `features/ambience/` may import `@/lib/tauri` only from `ambienceSettings.ts`.
 */

import { listInstances } from '@/lib/tauri';

export async function countInstances(): Promise<number> {
  return (await listInstances()).length;
}
