import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { pluginSurfaces } from './api';
import { usePlugins } from './PluginProvider';
import { PluginView } from './PluginView';
import type { SurfaceChoice } from './types';

/**
 * Renders whoever the user chose for a built-in surface — usually Agora.
 *
 * The rule this component exists to enforce: a plugin offering to replace a
 * surface does not get it. It joins a list in Settings, and `fallback` — the
 * launcher's own view — is what renders until someone picks otherwise. Core
 * decides; this only asks and draws.
 *
 * Every uncertain path resolves to the built-in. Backend unreachable, an
 * answer in an unexpected shape, the chosen plugin disabled or removed, the
 * plugin's own view throwing: all of them end with the user looking at Agora's
 * home screen rather than at nothing. That matters more here than anywhere
 * else in the plugin system, because this is the first thing the launcher
 * shows and a blank one reads as a broken install.
 */

export function PluginSurface({
  surface,
  fallback,
  args,
}: {
  surface: string;
  fallback: ReactNode;
  /**
   * Passed to the plugin's export. The instance overview sends
   * `{ instanceId }` so one view serves every instance.
   */
  args?: Record<string, unknown>;
}) {
  const { enabled, ready, refreshToken } = usePlugins();
  const [choice, setChoice] = useState<SurfaceChoice | null>(null);
  // Guards against an answer arriving after the component is gone, and against
  // two loads resolving in the wrong order.
  const generation = useRef(0);

  const load = useCallback(async () => {
    const mine = ++generation.current;
    try {
      const all = await pluginSurfaces();
      if (mine !== generation.current) return;
      const found = Array.isArray(all)
        ? all.find((entry) => entry && entry.surface === surface)
        : undefined;
      setChoice(found ?? null);
    } catch {
      // Deliberately silent. Nothing about this is worth a toast on every app
      // start, and the consequence — the built-in renders — is the default
      // anyway. The manager surfaces the real state.
      if (mine === generation.current) setChoice(null);
    }
  }, [surface]);

  useEffect(() => {
    if (!ready || !enabled) {
      setChoice(null);
      return;
    }
    void load();
  }, [ready, enabled, load, refreshToken]);

  const effective = choice?.effective;
  if (!enabled || !effective || !effective.pluginId || !effective.export) {
    return <>{fallback}</>;
  }

  return (
    <PluginView
      pluginId={effective.pluginId}
      exportName={effective.export}
      title={effective.title}
      args={args}
    />
  );
}
