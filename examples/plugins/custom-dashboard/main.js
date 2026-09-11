import { instances, storage } from 'agora';
export async function count() {
  const total = (await instances.list()).length;
  await storage.set('lastCount', total);
  return { total };
}
