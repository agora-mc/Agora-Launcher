import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  AlertTriangle,
  FolderOpen,
  Package,
  PowerOff,
  RefreshCw,
  ScrollText,
  Trash2,
} from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';
import { showToast } from '@/components/Toast';
import { formatError, getSetting, setSetting } from '@/lib/tauri';
import {
  addPluginDevelopmentFolder,
  disableAllPlugins,
  installPluginPackage,
  previewPluginFolder,
  previewPluginPackage,
  readPluginLog,
  setPluginEnabled,
  uninstallPlugin,
} from './api';
import { usePlugins } from './PluginProvider';
import { PluginSettingsForm } from './PluginSettingsForm';
import { PluginThemeSelect } from './PluginTheme';
import type { InstallPreview, PluginSummary } from './types';

/**
 * Settings → Plugins.
 *
 * Three jobs, in order of how often they matter: explaining the state of each
 * plugin in a sentence the user can act on, letting them turn one off, and —
 * when something has gone badly wrong — letting them turn everything off
 * without any plugin having to cooperate.
 */

const PLUGINS_ENABLED = 'plugins_enabled';
const NETWORK_PLUGINS_ENABLED = 'network_plugins_enabled';

export function PluginManager() {
  const { plugins, refresh, ready } = usePlugins();
  const [systemEnabled, setSystemEnabled] = useState(false);
  const [networkEnabled, setNetworkEnabled] = useState(false);
  const [preview, setPreview] = useState<{ preview: InstallPreview; path: string; development: boolean } | null>(null);
  const [busy, setBusy] = useState(false);
  const [logFor, setLogFor] = useState<{ id: string; lines: string[] } | null>(null);

  useEffect(() => {
    void (async () => {
      setSystemEnabled((await getSetting(PLUGINS_ENABLED)) === true);
      setNetworkEnabled((await getSetting(NETWORK_PLUGINS_ENABLED)) === true);
    })();
  }, []);

  const toggleSystem = useCallback(
    async (next: boolean) => {
      try {
        await setSetting(PLUGINS_ENABLED, next);
        setSystemEnabled(next);
        if (!next) {
          // Turning the system off should actually stop what is running, not
          // just hide it until the next restart.
          await disableAllPlugins();
        }
        await refresh();
        showToast(
          next
            ? 'Plugins are on. Restart Agora to load anything already installed.'
            : 'Plugins are off and everything installed has been stopped.',
        );
      } catch (e) {
        showToast(formatError(e), 'error');
      }
    },
    [refresh],
  );

  const toggleNetwork = useCallback(async (next: boolean) => {
    try {
      await setSetting(NETWORK_PLUGINS_ENABLED, next);
      setNetworkEnabled(next);
    } catch (e) {
      showToast(formatError(e), 'error');
    }
  }, []);

  const choosePackage = useCallback(async () => {
    try {
      const path = await invoke<string | null>('pick_open_file', {
        title: 'Choose a plugin package',
        extensions: ['zip'],
      });
      if (!path) return;
      setPreview({ preview: await previewPluginPackage(path), path, development: false });
    } catch (e) {
      showToast(formatError(e), 'error');
    }
  }, []);

  const chooseFolder = useCallback(async () => {
    try {
      const path = await invoke<string | null>('pick_directory', {
        title: 'Choose a plugin folder',
      });
      if (!path) return;
      setPreview({ preview: await previewPluginFolder(path), path, development: true });
    } catch (e) {
      showToast(formatError(e), 'error');
    }
  }, []);

  const confirmInstall = useCallback(async () => {
    if (!preview) return;
    setBusy(true);
    try {
      const summary = preview.development
        ? await addPluginDevelopmentFolder(preview.path, true)
        : await installPluginPackage(preview.path, true);
      setPreview(null);
      await refresh();
      showToast(`${summary.name} ${summary.version} installed. Restart Agora to start it.`);
    } catch (e) {
      showToast(formatError(e), 'error');
    } finally {
      setBusy(false);
    }
  }, [preview, refresh]);

  const toggle = useCallback(
    async (plugin: PluginSummary, next: boolean) => {
      try {
        await setPluginEnabled(plugin.id, next);
        await refresh();
        if (next) showToast(`${plugin.name} is enabled. Its views and commands start it when needed; restart Agora for startup events.`);
      } catch (e) {
        showToast(formatError(e), 'error');
      }
    },
    [refresh],
  );

  const remove = useCallback(
    async (plugin: PluginSummary) => {
      if (!window.confirm(`Remove ${plugin.name}?`)) return;
      // Deliberately a second, separate question. Removing a plugin is not the
      // same as saying "throw away everything I configured in it", and one
      // prompt covering both would get the wrong answer half the time.
      const purge = window.confirm(
        `Also delete ${plugin.name}'s saved settings and data?\n\n` +
          'Choose Cancel to keep them, in case you reinstall it later.',
      );
      try {
        await uninstallPlugin(plugin.id, purge);
        await refresh();
        showToast(purge ? `${plugin.name} and its data were removed.` : `${plugin.name} was removed; its data was kept.`);
      } catch (e) {
        showToast(formatError(e), 'error');
      }
    },
    [refresh],
  );

  const showLog = useCallback(async (plugin: PluginSummary) => {
    try {
      setLogFor({ id: plugin.id, lines: await readPluginLog(plugin.id, 200) });
    } catch (e) {
      showToast(formatError(e), 'error');
    }
  }, []);

  const panic = useCallback(async () => {
    if (!window.confirm('Turn off every plugin? You can switch them back on individually.')) return;
    try {
      const count = await disableAllPlugins();
      await refresh();
      showToast(count === 0 ? 'Nothing was running.' : `${count} plugin(s) turned off.`);
    } catch (e) {
      showToast(formatError(e), 'error');
    }
  }, [refresh]);

  return (
    <div className="space-y-6">
      <section className="space-y-3">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h3 className="text-base font-medium">Community plugins</h3>
            <p className="text-sm text-muted-foreground">
              Plugins add pages, panels, commands and checks to Agora. They are off by default,
              run with only the permissions you grant, and can be turned off at any time. Agora
              works the same with none installed. Permissions are the real boundary here — a
              plugin runs inside Agora, so only install ones you trust.
            </p>
          </div>
          <Switch
            checked={systemEnabled}
            onCheckedChange={(checked) => void toggleSystem(checked)}
            aria-label="Enable community plugins"
          />
        </div>

        {systemEnabled ? (
          <div className="flex items-start justify-between gap-4 rounded-lg border border-border bg-muted/30 px-4 py-3">
            <div>
              <div className="text-sm font-medium">Let plugins reach the internet</div>
              <p className="text-xs text-muted-foreground">
                Only to the hosts a plugin listed in its manifest and you approved at install
                time. Lockdown Mode blocks this regardless.
              </p>
            </div>
            <Switch
              checked={networkEnabled}
              onCheckedChange={(checked) => void toggleNetwork(checked)}
              aria-label="Allow plugin network access"
            />
          </div>
        ) : null}
      </section>

      {systemEnabled ? (
        <>
          <PluginThemeSelect />
          <div className="flex flex-wrap gap-2">
            <Button variant="secondary" size="sm" onClick={() => void choosePackage()}>
              <Package className="h-4 w-4" /> Install from file
            </Button>
            <Button variant="secondary" size="sm" onClick={() => void chooseFolder()}>
              <FolderOpen className="h-4 w-4" /> Load a folder
            </Button>
            <Button variant="ghost" size="sm" onClick={() => void refresh()}>
              <RefreshCw className="h-4 w-4" /> Refresh
            </Button>
            {plugins.length > 0 ? (
              <Button variant="ghost" size="sm" onClick={() => void panic()}>
                <PowerOff className="h-4 w-4" /> Turn all off
              </Button>
            ) : null}
          </div>

          {preview ? (
            <InstallPrompt
              preview={preview.preview}
              development={preview.development}
              busy={busy}
              onCancel={() => setPreview(null)}
              onConfirm={() => void confirmInstall()}
            />
          ) : null}

          {!ready ? (
            <p className="text-sm text-muted-foreground">Loading…</p>
          ) : plugins.length === 0 ? (
            <p className="rounded-lg border border-dashed border-border px-4 py-8 text-center text-sm text-muted-foreground">
              No plugins installed. Install one from a file, or point Agora at a folder you are
              working in.
            </p>
          ) : (
            <ul className="space-y-3">
              {plugins.map((plugin) => (
                <PluginRow
                  key={plugin.id}
                  plugin={plugin}
                  onToggle={(next) => void toggle(plugin, next)}
                  onRemove={() => void remove(plugin)}
                  onShowLog={() => void showLog(plugin)}
                />
              ))}
            </ul>
          )}

          {logFor ? (
            <div className="rounded-lg border border-border bg-card">
              <div className="flex items-center justify-between border-b border-border px-4 py-2">
                <div className="text-sm font-medium">
                  <ScrollText className="mr-2 inline h-4 w-4" />
                  {logFor.id}
                </div>
                <Button variant="ghost" size="sm" onClick={() => setLogFor(null)}>
                  Close
                </Button>
              </div>
              <pre className="max-h-64 overflow-auto px-4 py-3 text-xs leading-relaxed">
                {logFor.lines.length > 0 ? logFor.lines.join('\n') : 'This plugin has not logged anything.'}
              </pre>
            </div>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

function PluginRow({
  plugin,
  onToggle,
  onRemove,
  onShowLog,
}: {
  plugin: PluginSummary;
  onToggle: (next: boolean) => void;
  onRemove: () => void;
  onShowLog: () => void;
}) {
  const [showSettings, setShowSettings] = useState(false);
  const hasSettings = plugin.contributions.some((c) => c.kind === 'setting');
  const broken = plugin.status.state !== 'ready' && plugin.status.state !== 'disabled';

  return (
    <li className="rounded-lg border border-border bg-card p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{plugin.name}</span>
            <span className="text-xs text-muted-foreground">{plugin.version}</span>
            {plugin.development ? <Badge variant="outline">Development</Badge> : null}
            {plugin.running ? <Badge variant="secondary">Running</Badge> : null}
          </div>
          {plugin.description ? (
            <p className="text-sm text-muted-foreground">{plugin.description}</p>
          ) : null}
          <p
            className={
              broken
                ? 'flex items-center gap-1.5 text-sm text-amber-600 dark:text-amber-400'
                : 'text-sm text-muted-foreground'
            }
          >
            {broken ? <AlertTriangle className="h-4 w-4 shrink-0" aria-hidden /> : null}
            {plugin.statusText}
          </p>
          <div className="flex flex-wrap gap-1.5 pt-1">
            {plugin.capabilities.map((capability) => (
              <Badge key={capability} variant="outline" className="font-mono text-[10px]">
                {capability}
              </Badge>
            ))}
          </div>
          {plugin.declaredHosts.length > 0 ? (
            <p className="text-xs text-muted-foreground">
              Reaches: {plugin.declaredHosts.join(', ')}
            </p>
          ) : null}
          {plugin.droppedEvents > 0 ? (
            <p className="text-xs text-muted-foreground">
              Missed {plugin.droppedEvents} event(s) because it could not keep up.
            </p>
          ) : null}
          <p className="text-xs text-muted-foreground">
            {plugin.license}
            {plugin.sourceUrl ? ` · ${plugin.sourceUrl}` : ' · no source link given'}
          </p>
        </div>

        <div className="flex shrink-0 flex-col items-end gap-2">
          <Switch
            checked={plugin.enabled}
            onCheckedChange={onToggle}
            aria-label={`Enable ${plugin.name}`}
          />
          <div className="flex gap-1">
            {hasSettings ? (
              <Button variant="ghost" size="sm" onClick={() => setShowSettings((v) => !v)}>
                Settings
              </Button>
            ) : null}
            <Button variant="ghost" size="sm" onClick={onShowLog} aria-label="View log">
              <ScrollText className="h-4 w-4" />
            </Button>
            <Button variant="ghost" size="sm" onClick={onRemove} aria-label="Remove plugin">
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
        </div>
      </div>

      {showSettings ? (
        <div className="mt-4 border-t border-border pt-4">
          <PluginSettingsForm pluginId={plugin.id} />
        </div>
      ) : null}
    </li>
  );
}

function InstallPrompt({
  preview,
  development,
  busy,
  onCancel,
  onConfirm,
}: {
  preview: InstallPreview;
  development: boolean;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const blocked = preview.unsupportedCapabilities.length > 0;
  // Coerced: an older backend that predates this field would otherwise put
  // `undefined` where a list is expected, in a component whose whole job is
  // telling the user what they are agreeing to.
  const newlyRequested = Array.isArray(preview.addedCapabilities)
    ? preview.addedCapabilities
    : [];
  const newHosts = Array.isArray(preview.addedHosts) ? preview.addedHosts : [];

  return (
    <div className="rounded-lg border border-border bg-muted/30 p-4">
      <div className="space-y-1">
        <div className="font-medium">
          {preview.manifest.name} {preview.manifest.version}
        </div>
        <p className="text-sm text-muted-foreground">
          {preview.manifest.description ?? 'No description given.'}
        </p>
        <p className="text-xs text-muted-foreground">
          {preview.manifest.license}
          {preview.manifest.source ? ` · ${preview.manifest.source}` : ' · no source link given'}
          {development ? ' · loaded from a folder you are editing' : ''}
        </p>
      </div>

      {preview.replacesVersion ? (
        <p className="mt-3 text-sm">
          This replaces version {preview.replacesVersion}.
          {preview.migratesData
            ? ' It changes how it stores data — Agora will keep a copy of the old data first.'
            : ''}
        </p>
      ) : null}

      {/*
        On an update the full permission list is not the decision — the user
        already agreed to most of it. Naming only what is new is what makes a
        release that starts asking for more impossible to skim past.
      */}
      {preview.replacesVersion && (newlyRequested.length > 0 || newHosts.length > 0) ? (
        <p className="mt-2 flex items-start gap-2 text-sm text-amber-700 dark:text-amber-400">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
          This version asks for something it did not have before:{' '}
          {[...newlyRequested, ...newHosts.map((host) => `contact ${host}`)].join(', ')}.
        </p>
      ) : null}

      {preview.requiredCapabilities.length > 0 ? (
        <div className="mt-3">
          <div className="text-sm font-medium">It needs permission to:</div>
          <ul className="mt-1 space-y-1 text-sm text-muted-foreground">
            {preview.requiredCapabilities.map((capability) => (
              <li key={capability.name}>
                {capability.summary}
                {capability.isMutating ? (
                  <span className="ml-1 text-amber-600 dark:text-amber-400">(makes changes)</span>
                ) : null}
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {/*
        Optional capabilities are granted too — the plugin simply copes if they
        are missing in some future build. Listing only the required ones told
        the user "it asks for no permissions" while handing over the optional
        ones, which is the one sentence this dialog must never get wrong.
      */}
      {preview.optionalCapabilities.length > 0 ? (
        <div className="mt-3">
          <div className="text-sm font-medium">It will also use, if allowed:</div>
          <ul className="mt-1 space-y-1 text-sm text-muted-foreground">
            {preview.optionalCapabilities.map((capability) => (
              <li key={capability.name}>
                {capability.summary}
                {capability.isMutating ? (
                  <span className="ml-1 text-amber-600 dark:text-amber-400">(makes changes)</span>
                ) : null}
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {preview.requiredCapabilities.length === 0 &&
      preview.optionalCapabilities.length === 0 ? (
        <p className="mt-3 text-sm text-muted-foreground">It asks for no permissions.</p>
      ) : null}

      {preview.manifest.network?.hosts?.length ? (
        <p className="mt-2 text-sm text-muted-foreground">
          It may contact: {preview.manifest.network.hosts.join(', ')}
        </p>
      ) : null}

      {blocked ? (
        <p className="mt-3 flex items-start gap-2 text-sm text-destructive">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
          This plugin needs something this version of Agora does not provide:{' '}
          {preview.unsupportedCapabilities.join(', ')}. Check for an Agora update.
        </p>
      ) : null}

      <div className="mt-4 flex gap-2">
        <Button size="sm" onClick={onConfirm} disabled={busy || blocked}>
          {busy ? 'Installing…' : 'Install'}
        </Button>
        <Button size="sm" variant="ghost" onClick={onCancel} disabled={busy}>
          Cancel
        </Button>
      </div>
    </div>
  );
}
