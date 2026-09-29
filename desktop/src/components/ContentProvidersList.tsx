import { useCallback, useEffect, useState } from 'react';
import {
  formatError,
  listContentProviders,
  setContentProviderEnabled,
  type ProviderDescriptor,
} from '../lib/tauri';
import { showToast } from './Toast';

/**
 * Content sources contributed by plugins, each with the one switch that
 * controls it.
 *
 * A plugin provider's switch *is* its plugin's: turning it off disables the
 * plugin. Uninstalling stays in Plugins, where the rest of a plugin's
 * lifecycle lives. Agora's official providers keep their own toggles above.
 */
export function ContentProvidersList() {
  const [providers, setProviders] = useState<ProviderDescriptor[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(() => {
    listContentProviders()
      .then((list) => setProviders(Array.isArray(list) ? list : []))
      .catch(() => setProviders([]));
  }, []);

  useEffect(refresh, [refresh]);

  const pluginProviders = (providers ?? []).filter((p) => p.origin.kind === 'plugin');

  const toggle = async (provider: ProviderDescriptor, enabled: boolean) => {
    setBusy(provider.id);
    try {
      const list = await setContentProviderEnabled(provider.id, enabled);
      setProviders(Array.isArray(list) ? list : []);
    } catch (e) {
      showToast(formatError(e), 'error');
      refresh();
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="rounded-lg border border-border bg-card p-3 space-y-3">
      <div>
        <h4 className="text-sm font-medium">Plugin content sources</h4>
        <p className="text-xs text-muted-foreground mt-0.5">
          Plugins can add places to browse and install from. A plugin only suggests what to
          install: Agora downloads every file itself, checks it against the digest the source
          published, and labels what it installed with the source it came from. Files from hosts a
          source did not declare, or without a strong digest, count as unverified content.
        </p>
      </div>
      {providers === null ? (
        <p className="text-xs text-muted-foreground">Loading…</p>
      ) : pluginProviders.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          No plugin adds a content source. Install one from Plugins, with plugins switched on.
        </p>
      ) : (
        <ul className="space-y-2">
          {pluginProviders.map((provider) => (
            <li key={provider.id}>
              <label className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <span className="text-sm">{provider.title}</span>
                  <p className="text-xs text-muted-foreground">
                    {provider.description ? `${provider.description} · ` : ''}
                    from the plugin {provider.origin.kind === 'plugin' ? provider.origin.pluginId : ''}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {provider.downloadHosts.length > 0
                      ? `Downloads from ${provider.downloadHosts.join(', ')}`
                      : 'Declares no download hosts'}
                  </p>
                  {provider.enabled && provider.unavailableReason && (
                    <p className="text-xs text-amber-600 dark:text-amber-400">
                      {provider.unavailableReason}
                    </p>
                  )}
                </div>
                <input
                  type="checkbox"
                  aria-label={`${provider.title} content source`}
                  checked={provider.enabled}
                  disabled={busy === provider.id}
                  onChange={(e) => toggle(provider, e.target.checked)}
                  className="mt-0.5 h-5 w-5 shrink-0 accent-primary"
                />
              </label>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
