import { useEffect, useState } from 'react';
import { Button } from '@/components/ui/button';
import { showToast } from '@/components/Toast';
import { formatError } from '@/lib/tauri';
import { pluginInstanceOpened, runPluginCommand } from './api';
import { usePlugins } from './PluginProvider';
import { PluginView } from './PluginView';
import { PluginDiagnostic } from './PluginDiagnostics';
import type { ViewSource } from './types';

function ContributedView({ pluginId, title, view, instanceId }: {
  pluginId: string; title: string; view: ViewSource; instanceId?: string;
}) {
  return <PluginView pluginId={pluginId} title={title} exportName={view.export}
    args={instanceId ? { instanceId } : undefined} />;
}

export function PluginPage({ contributionId, onGoHome }: { contributionId: string; onGoHome: () => void }) {
  const { ready, ofKind, plugins } = usePlugins();
  const contribution = ofKind('page').find((entry) => entry.id === contributionId);
  const definition = plugins.find((plugin) => plugin.id === contribution?.pluginId)
    ?.definitions?.pages.find((page) => page.id === contribution?.localId);
  if (!ready) return <p role="status">Loading plugins…</p>;
  if (!contribution || !definition) return <div className="space-y-3 p-6">
    <h2 className="text-xl font-semibold">This plugin page is unavailable</h2>
    <p>The plugin may be disabled, removed, or unable to start. Check Services → Plugins in Settings.</p>
    <Button onClick={onGoHome}>Go home</Button>
  </div>;
  return <ContributedView key={contributionId} pluginId={contribution.pluginId}
    title={definition.title} view={definition.view} />;
}

export function PluginCommandButton({ pluginId, title, exportName, instanceId }: {
  pluginId: string; title: string; exportName: string; instanceId?: string;
}) {
  const [busy, setBusy] = useState(false);
  return <Button variant="secondary" disabled={busy} onClick={() => {
    setBusy(true);
    void runPluginCommand(pluginId, exportName, instanceId ? { instanceId } : null)
      .then(() => showToast(`${title} completed.`))
      .catch((error: unknown) => showToast(formatError(error), 'error'))
      .finally(() => setBusy(false));
  }}>{busy ? 'Running…' : title}</Button>;
}

export function PluginInstancePanels({ instanceId }: { instanceId: string }) {
  const { ofKind, plugins, enabled } = usePlugins();
  // Panels below already start their own plugin when they render. This is for
  // a plugin that declared `onInstanceOpened` and contributes nothing visible,
  // so nothing else would ever wake it.
  useEffect(() => {
    if (!enabled) return;
    void pluginInstanceOpened().catch(() => {
      // Best effort: a plugin that fails to wake is reported in Settings, and
      // the panels on this page must still render.
    });
  }, [enabled, instanceId]);
  const panels = ofKind('instance-panel');
  const commands = ofKind('command');
  return <>
    {ofKind('diagnostic').map((entry) => {
      const definition = plugins.find((plugin) => plugin.id === entry.pluginId)
        ?.definitions?.diagnostics.find((diagnostic) => diagnostic.id === entry.localId);
      return definition ? <PluginDiagnostic key={entry.id} pluginId={entry.pluginId}
        title={entry.title} exportName={definition.export} instanceId={instanceId} /> : null;
    })}
    {panels.map((panel) => {
      const definition = plugins.find((plugin) => plugin.id === panel.pluginId)
        ?.definitions?.instancePanels.find((entry) => entry.id === panel.localId);
      return definition ? <section key={panel.id} className="rounded-lg border border-border p-4">
        <h3 className="font-semibold">{panel.title}</h3>
        <ContributedView pluginId={panel.pluginId} title={panel.title} view={definition.view} instanceId={instanceId} />
      </section> : null;
    })}
    {commands.map((command) => {
      const definition = plugins.find((plugin) => plugin.id === command.pluginId)
        ?.definitions?.commands.find((entry) => entry.id === command.localId);
      return definition?.surfaces.includes('instance-context') ? <PluginCommandButton key={command.id}
        pluginId={command.pluginId} title={command.title} exportName={definition.export} instanceId={instanceId} /> : null;
    })}
  </>;
}
