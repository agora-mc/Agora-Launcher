import { instances, storage, ui } from 'agora';

export async function dashboard() {
  const all = await instances.list();
  const previous = await storage.get('instanceCount');
  return {
    title: 'Instance dashboard',
    blocks: [
      { type: 'stats', items: [
        { label: 'Instances', value: String(all.length) },
        { label: 'Remembered count', value: previous === null ? 'Not saved' : String(previous) },
      ] },
      { type: 'table', columns: [{ label: 'Instance' }, { label: 'Minecraft' }],
        rows: all.map((instance) => [{ type: 'text', text: instance.name }, { type: 'text', text: instance.minecraftVersion }]) },
      { type: 'actions', items: [{ id: 'remember-count', label: 'Remember this count', export: 'remember' }] },
    ],
  };
}

export async function remember() {
  const count = (await instances.list()).length;
  await storage.set('instanceCount', count);
  await ui.refresh();
  return { count };
}
