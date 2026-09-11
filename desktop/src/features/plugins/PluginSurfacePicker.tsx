import { useCallback, useEffect, useState } from 'react';
import { AlertTriangle } from 'lucide-react';
import { showToast } from '@/components/Toast';
import { formatError } from '@/lib/tauri';
import { pluginSurfaces, setPluginSurface } from './api';
import { usePlugins } from './PluginProvider';
import type { SurfaceChoice } from './types';

/**
 * Choosing who renders each built-in surface.
 *
 * This control is the reason a plugin's offer to replace something is only an
 * offer. Nothing a plugin declares changes what the launcher looks like until
 * a choice is made here, and Agora's own view is always the first option
 * rather than an afterthought at the bottom of the list.
 */

export function PluginSurfacePicker() {
  const { refresh, refreshToken } = usePlugins();
  const [surfaces, setSurfaces] = useState<SurfaceChoice[]>([]);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      const all = await pluginSurfaces();
      // Coerced rather than trusted: an older backend answers `null` here, and
      // a `null` reaching `.map` would take the settings page down.
      setSurfaces(Array.isArray(all) ? all.filter((entry) => entry && entry.surface) : []);
    } catch {
      setSurfaces([]);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load, refreshToken]);

  const choose = useCallback(
    async (surface: string, contributionId: string | null) => {
      setBusy(true);
      try {
        await setPluginSurface(surface, contributionId);
        await load();
        // The surface itself reads its choice through the provider's refresh
        // token, so bumping it is what makes the change visible immediately
        // rather than on the next navigation.
        await refresh();
      } catch (e) {
        showToast(formatError(e), 'error');
      } finally {
        setBusy(false);
      }
    },
    [load, refresh],
  );

  // A surface nobody has offered to render is not a choice, so showing it
  // would be a row of one permanently-selected option.
  const offered = surfaces.filter((surface) => surface.offers.length > 0);
  if (offered.length === 0) return null;

  return (
    <section className="space-y-3">
      <div>
        <h3 className="text-sm font-medium">Who draws each screen</h3>
        <p className="text-xs text-muted-foreground">
          A plugin can offer to replace one of Agora&apos;s own screens. It never takes one on
          its own — you pick here, and you can always come back.
        </p>
      </div>

      {offered.map((surface) => (
        <div
          key={surface.surface}
          className="rounded-lg border border-border bg-muted/30 px-4 py-3"
        >
          <div className="text-sm font-medium">{surface.title}</div>

          <div className="mt-2 space-y-1">
            <label className="flex items-start gap-2 text-sm">
              <input
                type="radio"
                className="mt-1"
                name={`surface-${surface.surface}`}
                checked={surface.selected === null}
                disabled={busy}
                onChange={() => void choose(surface.surface, null)}
              />
              <span>
                <span className="font-medium">Agora</span>
                <span className="block text-xs text-muted-foreground">
                  The built-in screen.
                </span>
              </span>
            </label>

            {surface.offers.map((offer) => (
              <label key={offer.id} className="flex items-start gap-2 text-sm">
                <input
                  type="radio"
                  className="mt-1"
                  name={`surface-${surface.surface}`}
                  checked={surface.selected === offer.id}
                  disabled={busy}
                  onChange={() => void choose(surface.surface, offer.id)}
                />
                <span>
                  <span className="font-medium">{offer.title}</span>
                  <span className="text-xs text-muted-foreground"> · {offer.pluginName}</span>
                  {offer.description ? (
                    <span className="block text-xs text-muted-foreground">
                      {offer.description}
                    </span>
                  ) : null}
                </span>
              </label>
            ))}
          </div>

          {/*
            Shown only when a choice exists and is not what renders. Saying so
            is the difference between "you chose Agora's screen" and "your
            choice is broken and we quietly did something else".
          */}
          {surface.fallbackReason ? (
            <p className="mt-2 flex items-start gap-2 text-xs text-amber-700 dark:text-amber-400">
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden />
              {surface.fallbackReason}
            </p>
          ) : null}
        </div>
      ))}
    </section>
  );
}
