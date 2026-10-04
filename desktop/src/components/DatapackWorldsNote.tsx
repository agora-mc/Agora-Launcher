import { useEffect, useState } from 'react';
import { datapackInstallNote } from '../lib/datapackWorlds';
import { listInstanceWorlds } from '../lib/tauri';

/**
 * Install-review line for a data pack. Minecraft only loads data packs from
 * inside a world, so say which worlds Agora will copy it into.
 */
export function DatapackWorldsNote({ instanceId }: { instanceId: string }) {
  const [worldCount, setWorldCount] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    listInstanceWorlds(instanceId)
      .then((worlds) => { if (!cancelled) setWorldCount(worlds.length); })
      .catch(() => { if (!cancelled) setWorldCount(null); });
    return () => { cancelled = true; };
  }, [instanceId]);

  if (worldCount === null) return null;
  return <p className="mt-3 text-xs text-muted-foreground" data-testid="datapack-worlds-note">{datapackInstallNote(worldCount)}</p>;
}
