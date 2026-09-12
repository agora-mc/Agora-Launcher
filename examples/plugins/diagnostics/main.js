import { content } from 'agora';

export async function check({ instanceId }) {
  const mods = await content.list(instanceId, 'mod');
  return { findings: mods.filter((mod) => !mod.enabled).map((mod) => ({
    id: `disabled:${mod.key}`, title: `${mod.displayName} is disabled`, severity: 'info',
    summary: 'This may be intentional. Enable it only if you want it in this instance.',
    evidence: [{ label: 'File', value: mod.filename }],
    repairs: [{ id: `enable:${mod.key}`, title: `Enable ${mod.displayName}`,
      actions: [{ action: 'enableContent', instanceId, key: mod.key }] }],
  })) };
}
