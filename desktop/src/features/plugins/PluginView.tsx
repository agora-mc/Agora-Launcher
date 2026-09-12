import { useCallback, useEffect, useState } from 'react';
import { AlertTriangle } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { formatError } from '@/lib/tauri';
import { renderPluginView, runPluginCommand } from './api';
import { usePlugins } from './PluginProvider';
import { PluginViewRenderer } from './PluginViewRenderer';
import type { ActionButton, ViewModel } from './types';

/**
 * Hosts one contributed view — a page or an instance panel.
 *
 * Failure is the interesting case. A plugin that throws, times out, or returns
 * something unrenderable gets an explicit panel naming the plugin and the
 * problem, with a retry. It does not get a blank screen, and it does not get
 * to take the surrounding page down with it: the launcher's own screens keep
 * working with a plugin broken, which is the same promise the runtime makes
 * one layer down.
 */

export interface PluginViewProps {
  pluginId: string;
  /** The plugin's exported function that builds the view. */
  exportName: string;
  /** Passed to the export; the instance panel sends the current instance. */
  args?: Record<string, unknown>;
  /** Shown above the view while the plugin is loading. */
  title?: string;
}

export function PluginView({ pluginId, exportName, args, title }: PluginViewProps) {
  const { refreshToken } = usePlugins();
  const [model, setModel] = useState<ViewModel | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [attempt, setAttempt] = useState(0);

  const argsKey = JSON.stringify(args ?? null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    void (async () => {
      try {
        const next = await renderPluginView(pluginId, exportName, args ?? null);
        if (!cancelled) setModel(next);
      } catch (e) {
        if (!cancelled) {
          setError(formatError(e));
          setModel(null);
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
    // `argsKey` rather than `args` so a caller passing a fresh object literal
    // each render does not re-invoke the plugin on every paint.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pluginId, exportName, argsKey, attempt, refreshToken]);

  const onAction = useCallback(
    async (action: ActionButton) => {
      setBusy(true);
      try {
        await runPluginCommand(pluginId, action.export, action.args ?? args ?? null);
        // The action almost certainly changed what the view is showing, so
        // re-render from the plugin rather than guessing at the new state.
        setAttempt((value) => value + 1);
      } catch (e) {
        setError(formatError(e));
      } finally {
        setBusy(false);
      }
    },
    [pluginId, args],
  );

  if (loading && !model) {
    return (
      <div className="p-6 text-sm text-muted-foreground">
        {title ? `Loading ${title}…` : 'Loading…'}
      </div>
    );
  }

  if (error && !model) {
    return (
      <div className="m-6 rounded-lg border border-destructive/40 bg-destructive/10 p-4">
        <div className="flex items-start gap-3">
          <AlertTriangle className="mt-0.5 h-5 w-5 shrink-0 text-destructive" aria-hidden />
          <div className="min-w-0 space-y-2">
            <div className="font-medium">This plugin view could not be shown</div>
            <p className="text-sm text-muted-foreground">
              <span className="font-mono text-xs">{pluginId}</span> reported: {error}
            </p>
            <p className="text-xs text-muted-foreground">
              Agora itself is unaffected. You can turn this plugin off in Settings → Plugins.
            </p>
            <Button size="sm" variant="secondary" onClick={() => setAttempt((v) => v + 1)}>
              Try again
            </Button>
          </div>
        </div>
      </div>
    );
  }

  if (!model) return null;

  return (
    <div className="p-6">
      {/* An error raised by an action, while an earlier view is still shown. */}
      {error ? (
        <div className="mb-4 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm">
          {error}
        </div>
      ) : null}
      <PluginViewRenderer model={model} onAction={onAction} busy={busy} />
    </div>
  );
}
