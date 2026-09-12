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
import { useConfirm } from '@/components/ui/confirm';
import { formatError, getSetting, setSetting } from '@/lib/tauri';
import {
  addPluginDevelopmentFolder,
  applyPluginUpdate,
  checkPluginUpdate,
  disableAllPlugins,
  installPluginPackage,
  previewPluginFolder,
  previewPluginPackage,
  readPluginLog,
  restorePluginData,
  setPluginEnabled,
  uninstallPlugin,
} from './api';
import { usePlugins } from './PluginProvider';
import { PluginSettingsForm } from './PluginSettingsForm';
import { PluginThemeSelect } from './PluginTheme';
import { PluginSurfacePicker } from './PluginSurfacePicker';
import type {
  CapabilityDescription,
  CheckpointSummary,
  InstallPreview,
  KeyFingerprint,
  PluginSummary,
  UpdateCheckRecord,
  UpdateOutcome,
  UpdateSourceSummary,
  UpdateVerdict,
} from './types';

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
const PLUGIN_UPDATES_ENABLED = 'plugin_updates_enabled';

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isPluginSummary(value: unknown): value is PluginSummary {
  return isRecord(value) && typeof value.id === 'string';
}

function stringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : [];
}

function isCapabilityDescription(value: unknown): value is CapabilityDescription {
  return (
    isRecord(value) &&
    typeof value.name === 'string' &&
    typeof value.summary === 'string' &&
    typeof value.isMutating === 'boolean'
  );
}

function isKeyFingerprint(value: unknown): value is KeyFingerprint {
  return isRecord(value) && typeof value.id === 'string' && typeof value.fingerprint === 'string';
}

function isUpdateSourceSummary(value: unknown): value is UpdateSourceSummary {
  return (
    isRecord(value) &&
    typeof value.url === 'string' &&
    typeof value.host === 'string' &&
    Array.isArray(value.keys) &&
    value.keys.every(isKeyFingerprint)
  );
}

function updateSourceForDisplay(value: unknown): UpdateSourceSummary | null {
  return isUpdateSourceSummary(value) ? value : null;
}

function isUpdateCheckRecord(value: unknown): value is UpdateCheckRecord {
  return isRecord(value) && typeof value.at === 'string' && typeof value.result === 'string';
}

function lastUpdateCheckForDisplay(value: unknown): UpdateCheckRecord | null {
  return isUpdateCheckRecord(value) ? value : null;
}

function isInstallPreview(value: unknown): value is InstallPreview {
  if (!isRecord(value) || !isRecord(value.manifest)) return false;
  return (
    typeof value.manifest.name === 'string' &&
    typeof value.manifest.version === 'string' &&
    typeof value.manifest.license === 'string' &&
    Array.isArray(value.requiredCapabilities) &&
    value.requiredCapabilities.every(isCapabilityDescription) &&
    Array.isArray(value.optionalCapabilities) &&
    value.optionalCapabilities.every(isCapabilityDescription) &&
    Array.isArray(value.unsupportedCapabilities) &&
    value.unsupportedCapabilities.every((item) => typeof item === 'string') &&
    (value.replacesVersion === null || value.replacesVersion === undefined || typeof value.replacesVersion === 'string') &&
    Array.isArray(value.addedCapabilities) &&
    value.addedCapabilities.every((item) => typeof item === 'string') &&
    Array.isArray(value.addedHosts) &&
    value.addedHosts.every((item) => typeof item === 'string') &&
    (value.updateSource === null || value.updateSource === undefined || isUpdateSourceSummary(value.updateSource)) &&
    typeof value.migratesData === 'boolean' &&
    typeof value.fileCount === 'number' &&
    typeof value.uncompressedBytes === 'number'
  );
}

function isUpdateVerdict(value: unknown): value is UpdateVerdict {
  if (!isRecord(value) || typeof value.state !== 'string') return false;
  if (value.state === 'upToDate' || value.state === 'noReleases') return true;
  if (value.state === 'available') {
    return (
      typeof value.from === 'string' &&
      typeof value.to === 'string' &&
      (value.notes === null || typeof value.notes === 'string') &&
      typeof value.url === 'string' &&
      typeof value.sha256 === 'string' &&
      typeof value.size === 'number'
    );
  }
  if (value.state === 'needsNewerHost') {
    return typeof value.latest === 'string' && typeof value.requires === 'string';
  }
  if (value.state === 'installedIsNewer') {
    return typeof value.installed === 'string' && typeof value.latest === 'string';
  }
  return false;
}

function isUpdateOutcome(value: unknown): value is UpdateOutcome {
  if (!isRecord(value) || typeof value.outcome !== 'string') return false;
  if (value.outcome === 'installed') return isRecord(value.plugin);
  if (value.outcome === 'needsConsent') return isInstallPreview(value.preview);
  return false;
}

export function PluginManager() {
  const { plugins, refresh, ready } = usePlugins();
  const safePlugins = Array.isArray(plugins) ? plugins.filter(isPluginSummary) : [];
  const [systemEnabled, setSystemEnabled] = useState(false);
  const [networkEnabled, setNetworkEnabled] = useState(false);
  const [updatesEnabled, setUpdatesEnabled] = useState(false);
  const [preview, setPreview] = useState<{ preview: InstallPreview; path: string; development: boolean } | null>(null);
  const [busy, setBusy] = useState(false);
  const [logFor, setLogFor] = useState<{ id: string; lines: string[] } | null>(null);

  useEffect(() => {
    void (async () => {
      setSystemEnabled((await getSetting(PLUGINS_ENABLED)) === true);
      setNetworkEnabled((await getSetting(NETWORK_PLUGINS_ENABLED)) === true);
      setUpdatesEnabled((await getSetting(PLUGIN_UPDATES_ENABLED)) === true);
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

  const toggleUpdates = useCallback(async (next: boolean) => {
    try {
      await setSetting(PLUGIN_UPDATES_ENABLED, next);
      setUpdatesEnabled(next);
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
      const nextPreview = await previewPluginPackage(path);
      if (!isInstallPreview(nextPreview)) {
        showToast('Agora did not return a usable plugin preview.', 'error');
        return;
      }
      setPreview({ preview: nextPreview, path, development: false });
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
      const nextPreview = await previewPluginFolder(path);
      if (!isInstallPreview(nextPreview)) {
        showToast('Agora did not return a usable plugin preview.', 'error');
        return;
      }
      setPreview({ preview: nextPreview, path, development: true });
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
      if (!isRecord(summary) || typeof summary.name !== 'string' || typeof summary.version !== 'string') {
        showToast('Agora did not return a usable installed plugin.', 'error');
        return;
      }
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
      const lines = await readPluginLog(plugin.id, 200);
      setLogFor({ id: plugin.id, lines: stringArray(lines) });
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
          <div className="space-y-2">
            <div className="flex items-start justify-between gap-4 rounded-lg border border-border bg-muted/30 px-4 py-3">
              <div>
                <div className="text-sm font-medium">Check for plugin updates automatically</div>
                <p className="text-xs text-muted-foreground">
                  Controls background checks for signed plugin update documents. A manual check
                  remains available per plugin.
                </p>
              </div>
              <Switch
                checked={updatesEnabled}
                onCheckedChange={(checked) => void toggleUpdates(checked)}
                aria-label="Check for plugin updates automatically"
              />
            </div>

            <div className="flex items-start justify-between gap-4 rounded-lg border border-border bg-muted/30 px-4 py-3">
              <div>
                <div className="text-sm font-medium">Let plugins reach the network</div>
                <p className="text-xs text-muted-foreground">
                  Controls network access from plugin code, limited to the hosts a plugin listed
                  in its manifest and you approved at install time. Lockdown Mode blocks this
                  regardless.
                </p>
              </div>
              <Switch
                checked={networkEnabled}
                onCheckedChange={(checked) => void toggleNetwork(checked)}
                aria-label="Let plugins reach the network"
              />
            </div>
          </div>
        ) : null}
      </section>

      {systemEnabled ? (
        <>
          <PluginThemeSelect />
          <PluginSurfacePicker />
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
            {safePlugins.length > 0 ? (
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
          ) : safePlugins.length === 0 ? (
            <p className="rounded-lg border border-dashed border-border px-4 py-8 text-center text-sm text-muted-foreground">
              No plugins installed. Install one from a file, or point Agora at a folder you are
              working in.
            </p>
          ) : (
            <ul className="space-y-3">
              {safePlugins.map((plugin) => (
                <PluginRow
                  key={plugin.id}
                  plugin={plugin}
                  onRefresh={refresh}
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
  onRefresh,
  onToggle,
  onRemove,
  onShowLog,
}: {
  plugin: PluginSummary;
  onRefresh: () => Promise<void>;
  onToggle: (next: boolean) => void;
  onRemove: () => void;
  onShowLog: () => void;
}) {
  const [showSettings, setShowSettings] = useState(false);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [updateVerdict, setUpdateVerdict] = useState<UpdateVerdict | null>(null);
  const [updatePreview, setUpdatePreview] = useState<InstallPreview | null>(null);
  const pluginId = typeof plugin.id === 'string' ? plugin.id : 'unknown-plugin';
  const pluginName = typeof plugin.name === 'string' ? plugin.name : 'Unnamed plugin';
  const pluginVersion = typeof plugin.version === 'string' ? plugin.version : 'unknown version';
  const pluginDescription = typeof plugin.description === 'string' ? plugin.description : null;
  const pluginLicense = typeof plugin.license === 'string' ? plugin.license : 'Unknown license';
  const pluginSource = typeof plugin.sourceUrl === 'string' ? plugin.sourceUrl : null;
  const pluginStatusText = typeof plugin.statusText === 'string' ? plugin.statusText : 'Status unavailable.';
  const contributions = Array.isArray(plugin.contributions) ? plugin.contributions : [];
  const capabilities = stringArray(plugin.capabilities);
  const declaredHosts = stringArray(plugin.declaredHosts);
  const updateSource = updateSourceForDisplay(plugin.updateSource);
  const lastUpdateCheck = lastUpdateCheckForDisplay(plugin.lastUpdateCheck);
  // Coerced, like everything else arriving from a command that an older
  // backend answers with `null`.
  const restorable =
    isRecord(plugin.restorableData) && typeof plugin.restorableData.entryCount === 'number'
      ? (plugin.restorableData as unknown as CheckpointSummary)
      : null;
  const [restoreBusy, setRestoreBusy] = useState(false);
  const { confirm } = useConfirm();
  const hasSettings = contributions.some((contribution) => contribution?.kind === 'setting');
  const statusState = isRecord(plugin.status) ? plugin.status.state : null;
  const broken = statusState !== 'ready' && statusState !== 'disabled';
  const droppedEvents = typeof plugin.droppedEvents === 'number' ? plugin.droppedEvents : 0;

  // Never automatic. A saved copy is taken when an update changes how a plugin
  // stores data; going back to it also discards whatever the plugin has
  // written since, and only the user knows whether that is a loss. So the
  // launcher offers, names what it would put back, and asks.
  const restoreData = useCallback(async () => {
    if (!restorable) return;
    const agreed = await confirm({
      title: `Restore saved data for ${plugin.name}?`,
      body: (
        <>
          This puts back the {restorable.entryCount} stored{' '}
          {restorable.entryCount === 1 ? 'entry' : 'entries'} saved before version{' '}
          {restorable.fromVersion}. Anything {plugin.name} has stored since then is discarded.
        </>
      ),
      confirmLabel: 'Restore',
      tone: 'danger',
    });
    if (!agreed) return;
    setRestoreBusy(true);
    try {
      const restored = await restorePluginData(plugin.id);
      showToast(`Restored ${restored} stored entries for ${plugin.name}.`);
      await onRefresh();
    } catch (e) {
      showToast(formatError(e), 'error');
    } finally {
      setRestoreBusy(false);
    }
  }, [restorable, plugin.id, plugin.name, confirm, onRefresh]);

  const checkForUpdate = useCallback(async () => {
    setUpdateBusy(true);
    setUpdateVerdict(null);
    setUpdatePreview(null);
    try {
      const result = await checkPluginUpdate(pluginId);
      if (!isUpdateVerdict(result)) {
        showToast('Agora did not return a usable update result.', 'error');
        return;
      }
      setUpdateVerdict(result);
      await onRefresh();
    } catch (e) {
      showToast(formatError(e), 'error');
    } finally {
      setUpdateBusy(false);
    }
  }, [onRefresh, pluginId]);

  const applyUpdate = useCallback(
    async (acceptCapabilities: boolean) => {
      setUpdateBusy(true);
      try {
        const result = await applyPluginUpdate(pluginId, acceptCapabilities);
        if (!isUpdateOutcome(result)) {
          showToast('Agora did not return a usable update outcome.', 'error');
          return;
        }
        if (result.outcome === 'needsConsent') {
          setUpdatePreview(result.preview);
          return;
        }
        if (result.outcome === 'installed') {
          setUpdatePreview(null);
          setUpdateVerdict(null);
          await onRefresh();
          showToast('Plugin update installed.');
        }
      } catch (e) {
        showToast(formatError(e), 'error');
      } finally {
        setUpdateBusy(false);
      }
    },
    [onRefresh, pluginId],
  );

  return (
    <li className="rounded-lg border border-border bg-card p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{pluginName}</span>
            <span className="text-xs text-muted-foreground">{pluginVersion}</span>
            {plugin.development === true ? <Badge variant="outline">Development</Badge> : null}
            {plugin.running === true ? <Badge variant="secondary">Running</Badge> : null}
          </div>
          {pluginDescription ? (
            <p className="text-sm text-muted-foreground">{pluginDescription}</p>
          ) : null}
          <p
            className={
              broken
                ? 'flex items-center gap-1.5 text-sm text-amber-600 dark:text-amber-400'
                : 'text-sm text-muted-foreground'
            }
          >
            {broken ? <AlertTriangle className="h-4 w-4 shrink-0" aria-hidden /> : null}
            {pluginStatusText}
          </p>
          <div className="flex flex-wrap gap-1.5 pt-1">
            {capabilities.map((capability) => (
              <Badge key={capability} variant="outline" className="font-mono text-[10px]">
                {capability}
              </Badge>
            ))}
          </div>
          {declaredHosts.length > 0 ? (
            <p className="text-xs text-muted-foreground">
              Reaches: {declaredHosts.join(', ')}
            </p>
          ) : null}
          {updateSource ? (
            <div className="space-y-1 text-xs text-muted-foreground">
              <p>Checks updates at: {updateSource.host}</p>
              {updateSource.keys.length > 0 ? (
                <div>
                  <div>Publisher key fingerprints:</div>
                  <ul className="list-inside list-disc">
                    {updateSource.keys.map((key) => (
                      <li key={`${key.id}:${key.fingerprint}`}>
                        {key.id}: <span className="font-mono">{key.fingerprint}</span>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : (
                <p>No pinned publisher key fingerprints are recorded.</p>
              )}
              <p>
                A fingerprint means something only if compared against a key the author published
                somewhere you already trust. It is not by itself a verification step.
              </p>
            </div>
          ) : null}
          {lastUpdateCheck ? (
            <p className="text-xs text-muted-foreground">
              Last update check: {lastUpdateCheck.at} — {lastUpdateCheck.result}
            </p>
          ) : null}
          {droppedEvents > 0 ? (
            <p className="text-xs text-muted-foreground">
              Missed {droppedEvents} event(s) because it could not keep up.
            </p>
          ) : null}
          <p className="text-xs text-muted-foreground">
            {pluginLicense}
            {pluginSource ? ` · ${pluginSource}` : ' · no source link given'}
          </p>
        </div>

        <div className="flex shrink-0 flex-col items-end gap-2">
          <Switch
            checked={plugin.enabled === true}
            onCheckedChange={onToggle}
            aria-label={`Enable ${pluginName}`}
          />
          <div className="flex gap-1">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void checkForUpdate()}
              disabled={
                updateBusy ||
                updatePreview !== null ||
                plugin.development === true ||
                updateSource === null
              }
              title={
                plugin.development === true
                  ? 'Development folders are not updated.'
                  : updateSource === null
                    ? 'This plugin has no pinned update source.'
                    : undefined
              }
            >
              {updateBusy ? 'Checking…' : 'Check for updates'}
            </Button>
            {restorable ? (
              <Button
                variant="ghost"
                size="sm"
                disabled={restoreBusy}
                onClick={() => void restoreData()}
              >
                {restoreBusy ? 'Restoring…' : 'Restore saved data'}
              </Button>
            ) : null}
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
          <PluginSettingsForm pluginId={pluginId} />
        </div>
      ) : null}

      {updateVerdict ? (
        <UpdateResult
          verdict={updateVerdict}
          busy={updateBusy}
          onApply={() => void applyUpdate(false)}
        />
      ) : null}

      {updatePreview ? (
        <div className="mt-4 border-t border-border pt-4">
          <InstallPrompt
            preview={updatePreview}
            development={false}
            busy={updateBusy}
            confirmLabel="Update"
            busyLabel="Updating…"
            onCancel={() => setUpdatePreview(null)}
            onConfirm={() => void applyUpdate(true)}
          />
        </div>
      ) : null}
    </li>
  );
}

function UpdateResult({
  verdict,
  busy,
  onApply,
}: {
  verdict: UpdateVerdict;
  busy: boolean;
  onApply: () => void;
}) {
  if (verdict.state === 'upToDate') {
    return <p className="mt-3 text-sm text-muted-foreground">Up to date</p>;
  }
  if (verdict.state === 'available') {
    return (
      <div className="mt-3 space-y-2 rounded-md border border-border bg-muted/20 p-3 text-sm">
        <p>
          Version {verdict.to} is available (installed version {verdict.from}).
        </p>
        {verdict.notes ? (
          <p className="whitespace-pre-wrap text-muted-foreground">Release notes: {verdict.notes}</p>
        ) : null}
        <Button size="sm" onClick={onApply} disabled={busy}>
          Update to {verdict.to}
        </Button>
      </div>
    );
  }
  if (verdict.state === 'needsNewerHost') {
    return (
      <p className="mt-3 text-sm text-amber-700 dark:text-amber-400">
        Version {verdict.latest} is available, but it needs a newer version of Agora ({verdict.requires}).
      </p>
    );
  }
  if (verdict.state === 'installedIsNewer') {
    return (
      <p className="mt-3 text-sm text-muted-foreground">
        Installed version {verdict.installed} is newer than the publisher&apos;s latest listed version {verdict.latest}.
      </p>
    );
  }
  if (verdict.state === 'noReleases') {
    return <p className="mt-3 text-sm text-muted-foreground">The publisher lists no releases.</p>;
  }
  return null;
}

function InstallPrompt({
  preview,
  development,
  busy,
  confirmLabel = 'Install',
  busyLabel = 'Installing…',
  onCancel,
  onConfirm,
}: {
  preview: InstallPreview;
  development: boolean;
  busy: boolean;
  confirmLabel?: string;
  busyLabel?: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const requiredCapabilities = Array.isArray(preview.requiredCapabilities)
    ? preview.requiredCapabilities.filter(isCapabilityDescription)
    : [];
  const optionalCapabilities = Array.isArray(preview.optionalCapabilities)
    ? preview.optionalCapabilities.filter(isCapabilityDescription)
    : [];
  const unsupportedCapabilities = stringArray(preview.unsupportedCapabilities);
  // Coerced: an older backend that predates this field would otherwise put
  // `undefined` where a list is expected, in a component whose whole job is
  // telling the user what they are agreeing to.
  const newlyRequested = Array.isArray(preview.addedCapabilities)
    ? stringArray(preview.addedCapabilities)
    : [];
  const newHosts = Array.isArray(preview.addedHosts) ? stringArray(preview.addedHosts) : [];
  const manifestName = typeof preview.manifest?.name === 'string' ? preview.manifest.name : 'Plugin';
  const manifestVersion =
    typeof preview.manifest?.version === 'string' ? preview.manifest.version : 'unknown version';
  const manifestDescription =
    typeof preview.manifest?.description === 'string'
      ? preview.manifest.description
      : 'No description given.';
  const manifestLicense =
    typeof preview.manifest?.license === 'string' ? preview.manifest.license : 'Unknown license';
  const manifestSource = typeof preview.manifest?.source === 'string' ? preview.manifest.source : null;
  const manifestNetworkValue: unknown = preview.manifest?.network;
  const manifestNetwork = isRecord(manifestNetworkValue)
    ? stringArray(manifestNetworkValue.hosts)
    : [];
  const blocked = unsupportedCapabilities.length > 0;

  return (
    <div className="rounded-lg border border-border bg-muted/30 p-4">
      <div className="space-y-1">
        <div className="font-medium">
          {manifestName} {manifestVersion}
        </div>
        <p className="text-sm text-muted-foreground">
          {manifestDescription}
        </p>
        <p className="text-xs text-muted-foreground">
          {manifestLicense}
          {manifestSource ? ` · ${manifestSource}` : ' · no source link given'}
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

      {requiredCapabilities.length > 0 ? (
        <div className="mt-3">
          <div className="text-sm font-medium">It needs permission to:</div>
          <ul className="mt-1 space-y-1 text-sm text-muted-foreground">
            {requiredCapabilities.map((capability) => (
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
      {optionalCapabilities.length > 0 ? (
        <div className="mt-3">
          <div className="text-sm font-medium">It will also use, if allowed:</div>
          <ul className="mt-1 space-y-1 text-sm text-muted-foreground">
            {optionalCapabilities.map((capability) => (
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

      {requiredCapabilities.length === 0 && optionalCapabilities.length === 0 ? (
        <p className="mt-3 text-sm text-muted-foreground">It asks for no permissions.</p>
      ) : null}

      {manifestNetwork.length > 0 ? (
        <p className="mt-2 text-sm text-muted-foreground">
          It may contact: {manifestNetwork.join(', ')}
        </p>
      ) : null}

      {blocked ? (
        <p className="mt-3 flex items-start gap-2 text-sm text-destructive">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
          This plugin needs something this version of Agora does not provide:{' '}
          {unsupportedCapabilities.join(', ')}. Check for an Agora update.
        </p>
      ) : null}

      <div className="mt-4 flex gap-2">
        <Button size="sm" onClick={onConfirm} disabled={busy || blocked}>
          {busy ? busyLabel : confirmLabel}
        </Button>
        <Button size="sm" variant="ghost" onClick={onCancel} disabled={busy}>
          Cancel
        </Button>
      </div>
    </div>
  );
}
