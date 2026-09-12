import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { listen } from '@tauri-apps/api/event';
import { showToast } from '@/components/Toast';
import { formatError } from '@/lib/tauri';
import {
  listPluginContributions,
  listPlugins,
  pluginsEnabled,
  startPlugins,
} from './api';
import type {
  NamespacedContribution,
  PluginNotification,
  PluginRefresh,
  PluginSummary,
} from './types';

/**
 * The one place the app learns what plugins have contributed.
 *
 * Everything downstream — the sidebar, the palette, the instance editor, the
 * settings page — reads from here rather than calling the backend itself, so
 * there is a single answer to "what is installed right now" and a single
 * refresh that updates all of them together.
 *
 * When switched off, management metadata remains readable but no plugin
 * script is started and no plugin event listener is attached.
 */

interface PluginContextValue {
  /** Whether the user has turned the plugin system on. */
  enabled: boolean;
  /** False until the first load finishes, so consumers can avoid flicker. */
  ready: boolean;
  plugins: PluginSummary[];
  contributions: NamespacedContribution[];
  /** Contributions of one kind, in a stable order. */
  ofKind: (kind: NamespacedContribution['kind']) => NamespacedContribution[];
  refresh: () => Promise<void>;
  /** Bumped when a plugin asks for its views to re-render. */
  refreshToken: number;
}

const PluginContext = createContext<PluginContextValue>({
  enabled: false,
  ready: false,
  plugins: [],
  contributions: [],
  ofKind: () => [],
  refresh: async () => {},
  refreshToken: 0,
});

export function usePlugins(): PluginContextValue {
  return useContext(PluginContext);
}

export function PluginProvider({ children }: { children: ReactNode }) {
  const [enabled, setEnabled] = useState(false);
  const [ready, setReady] = useState(false);
  const [plugins, setPlugins] = useState<PluginSummary[]>([]);
  const [contributions, setContributions] = useState<NamespacedContribution[]>([]);
  const [refreshToken, setRefreshToken] = useState(0);
  // Guards against a refresh that resolves after the component unmounts, and
  // against two refreshes racing to set state in the wrong order.
  const generation = useRef(0);

  const refresh = useCallback(async () => {
    const mine = ++generation.current;
    try {
      const on = (await pluginsEnabled()) === true;
      const summaries = await listPlugins();
      const contributed = on ? await listPluginContributions() : [];
      if (mine !== generation.current) return;
      // Coerced rather than trusted. This provider sits above the whole app, so
      // a backend that answers with something other than a list — an older
      // build without these commands, a transport that resolves to `null` —
      // would otherwise put a non-array into state and take the entire window
      // down on the next `.find`, in a component that has nothing to do with
      // plugins.
      setPlugins(Array.isArray(summaries) ? summaries : []);
      setEnabled(on);
      setContributions(Array.isArray(contributed) ? contributed : []);
    } catch (e) {
      if (mine !== generation.current) return;
      setPlugins([]);
      setContributions([]);
      // A failure here means the manager shows nothing, which is bad but not
      // worth a toast on every app start; the settings page surfaces it.
      showToast(`Could not read plugins: ${formatError(e)}`, 'error');
    }
  }, []);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      let on = false;
      try {
        // Only an explicit `true` turns the subsystem on. Anything else — a
        // rejection, a `null` from a build without this command — reads as off,
        // which is also the default the user has not changed.
        on = (await pluginsEnabled()) === true;
      } catch {
        on = false;
      }
      if (cancelled) return;
      setEnabled(on);
      if (!on) {
        await refresh();
        setReady(true);
        return;
      }

      try {
        const started = await startPlugins();
        const failures = Array.isArray(started) ? started : [];
        if (!cancelled && failures.length > 0) {
          // Said once, plainly. The alternative is a user noticing that a
          // page they installed is simply not there.
          const names = failures.map((failure) => failure.pluginId).join(', ');
          showToast(
            failures.length === 1
              ? `The plugin ${names} could not start. See Settings → Plugins.`
              : `${failures.length} plugins could not start: ${names}`,
            'error',
          );
        }
      } catch (e) {
        if (!cancelled) showToast(formatError(e), 'error');
      }

      if (!cancelled) {
        await refresh();
        setReady(true);
      }
    })();

    return () => {
      cancelled = true;
      generation.current++;
    };
  }, [refresh]);

  // A plugin asking its own view to re-render, and a plugin raising a toast.
  useEffect(() => {
    if (!enabled) return;
    const unlisten: Array<Promise<() => void>> = [
      listen<PluginRefresh>('plugin-refresh', () => {
        setRefreshToken((token) => token + 1);
      }),
      listen<PluginNotification>('plugin-notify', (event) => {
        const { pluginId, tone, message } = event.payload;
        // Attributed to the plugin: a plugin must not be able to raise a
        // notification that reads as though Agora said it.
        showToast(`${pluginId}: ${message}`, tone === 'danger' ? 'error' : 'success');
      }),
    ];
    return () => {
      for (const pending of unlisten) void pending.then((off) => off());
    };
  }, [enabled]);

  const ofKind = useCallback(
    (kind: NamespacedContribution['kind']) =>
      contributions.filter((contribution) => contribution.kind === kind),
    [contributions],
  );

  const value = useMemo(
    () => ({ enabled, ready, plugins, contributions, ofKind, refresh, refreshToken }),
    [enabled, ready, plugins, contributions, ofKind, refresh, refreshToken],
  );

  return <PluginContext.Provider value={value}>{children}</PluginContext.Provider>;
}
