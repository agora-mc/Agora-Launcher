// Replacing a built-in surface.
//
// Declaring this contribution does not take the home screen over. It puts
// "Compact" on a list in Settings, and Agora's own home screen is what renders
// until someone chooses this one — so installing this plugin cannot change
// what the launcher looks like without a deliberate act.
//
// The same rule protects the user afterwards: if this plugin is disabled,
// removed, or fails, Agora renders its own home screen again and says why
// rather than leaving a blank page.

import { instances, launch } from 'agora';

/** The most recent launch across every instance, or null if nothing has run. */
async function mostRecentLaunch(all) {
  // `launch:read` is optional, so this has to work without it. A capability
  // the user declined is not an error, it is less detail — a plugin that
  // throws here would be punishing them for saying no.
  let newest = null;
  for (const instance of all) {
    let history;
    try {
      history = await launch.history({ instanceId: instance.id, limit: 1 });
    } catch {
      return null;
    }
    const entry = history[0];
    if (!entry) continue;
    if (!newest || entry.startedAt > newest.startedAt) {
      newest = { ...entry, instanceName: instance.name };
    }
  }
  return newest;
}

export async function home() {
  const all = await instances.list();
  const newest = await mostRecentLaunch(all);

  const blocks = [];

  if (newest) {
    blocks.push({
      type: 'status',
      tone: 'info',
      title: `Last played: ${newest.instanceName}`,
      message: newest.startedAt,
    });
  }

  blocks.push({
    type: 'stats',
    items: [
      { label: 'Instances', value: String(all.length) },
      {
        label: 'Modded',
        value: String(all.filter((instance) => instance.loader !== 'vanilla').length),
      },
    ],
  });

  blocks.push({
    type: 'table',
    columns: [{ label: 'Instance' }, { label: 'Minecraft' }, { label: 'Loader' }],
    rows: all.map((instance) => [
      { type: 'text', text: instance.name },
      { type: 'text', text: instance.minecraftVersion },
      { type: 'text', text: instance.loader },
    ]),
  });

  return { title: 'Home', blocks };
}
