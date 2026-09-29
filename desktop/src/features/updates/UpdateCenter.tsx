import { useState } from 'react';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { formatError, isPortableMode, restartApp } from '../../lib/tauri';
import {
  applyPluginUpdate,
  checkPluginUpdate,
  listPlugins,
  pluginsEnabled,
} from '../plugins/api';
import { showToast } from '../../components/Toast';
import { useConfirm } from '@/components/ui/confirm';

/**
 * One place to see every update — Agora itself and every plugin, including
 * the ones that add content sources — and one button to apply them.
 *
 * It owns no update logic. Agora's own update goes through the Tauri updater,
 * as it always has; each plugin goes through the plugin subsystem's own check
 * and apply, which verifies the signed update document, the package hash and
 * API compatibility. That subsystem is the authority: this panel only asks it.
 *
 * "Update all" never accepts a permission change on anyone's behalf. A plugin
 * update that asks for a capability or network host the installed version did
 * not have is left for the user to review in Plugins, exactly as it would be
 * if they had pressed its own Update button.
 */

type PluginRow = {
  id: string;
  name: string;
  from: string;
  to: string;
  state: 'available' | 'installing' | 'installed' | 'needs-consent' | 'failed';
  message?: string;
};

type Found = {
  app: Update | null;
  appSkipped?: string;
  /** Agora's own check failed: never shown as "up to date". */
  appError?: string;
  plugins: PluginRow[];
  pluginErrors: string[];
};

export function UpdateCenter() {
  const { confirm } = useConfirm();
  const [checking, setChecking] = useState(false);
  const [applying, setApplying] = useState(false);
  const [found, setFound] = useState<Found | null>(null);

  const checkEverything = async () => {
    setChecking(true);
    try {
      const portable = await isPortableMode().catch(() => false);
      const [app, plugins] = await Promise.all([
        portable
          ? Promise.resolve({ update: null, error: undefined })
          : check().then(
              (update) => ({ update, error: undefined }),
              (e: unknown) => ({ update: null, error: formatError(e) }),
            ),
        (async () => {
          const rows: PluginRow[] = [];
          const errors: string[] = [];
          if (!(await pluginsEnabled().catch(() => false))) return { rows, errors };
          const installed = await listPlugins();
          await Promise.all(
            installed
              .filter((plugin) => plugin.updateSource)
              .map(async (plugin) => {
                try {
                  const verdict = await checkPluginUpdate(plugin.id);
                  if (verdict?.state === 'available') {
                    rows.push({
                      id: plugin.id,
                      name: plugin.name,
                      from: verdict.from,
                      to: verdict.to,
                      state: 'available',
                    });
                  }
                } catch (e) {
                  errors.push(`${plugin.name}: ${formatError(e)}`);
                }
              }),
          );
          rows.sort((a, b) => a.name.localeCompare(b.name));
          return { rows, errors };
        })(),
      ]);
      setFound({
        app: app.update?.available ? app.update : null,
        appError: app.error,
        appSkipped: portable
          ? 'Portable copies update by replacing the executable from a new portable ZIP.'
          : undefined,
        plugins: plugins.rows,
        pluginErrors: plugins.errors,
      });
    } catch (e) {
      showToast(formatError(e), 'error');
    } finally {
      setChecking(false);
    }
  };

  const setRow = (id: string, patch: Partial<PluginRow>) =>
    setFound((current) =>
      current
        ? {
            ...current,
            plugins: current.plugins.map((row) => (row.id === id ? { ...row, ...patch } : row)),
          }
        : current,
    );

  const updateAll = async () => {
    if (!found || applying) return;
    setApplying(true);
    try {
      // Plugins first: Agora's own update ends in a restart, and anything not
      // done by then would simply not happen.
      for (const row of found.plugins.filter((r) => r.state === 'available')) {
        setRow(row.id, { state: 'installing' });
        try {
          const outcome = await applyPluginUpdate(row.id, false);
          if (outcome?.outcome === 'needsConsent') {
            setRow(row.id, {
              state: 'needs-consent',
              message: 'Asks for new permissions — review it in Plugins.',
            });
          } else {
            setRow(row.id, { state: 'installed' });
          }
        } catch (e) {
          setRow(row.id, { state: 'failed', message: formatError(e) });
        }
      }
      if (found.app) {
        const ok = await confirm({
          title: `Install Agora ${found.app.version} and restart?`,
          body: found.app.body ?? undefined,
          confirmLabel: 'Install and restart',
        });
        if (ok) {
          try {
            await found.app.downloadAndInstall();
            await restartApp();
          } catch (e) {
            showToast(`Could not install the Agora update: ${formatError(e)}`, 'error');
          }
        }
      }
    } finally {
      setApplying(false);
    }
  };

  const pending = found
    ? found.plugins.filter((row) => row.state === 'available').length + (found.app ? 1 : 0)
    : 0;

  return (
    <div className="rounded-lg border border-border bg-muted/30 p-3 space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h4 className="text-sm font-medium">Agora and plugins</h4>
          <p className="text-xs text-muted-foreground">
            Check Agora and every plugin — including plugins that add content sources — at once.
          </p>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={checkEverything}
            disabled={checking || applying}
            className="rounded-lg border border-input px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50"
          >
            {checking ? 'Checking…' : 'Check everything'}
          </button>
          {found && pending > 0 && (
            <button
              type="button"
              onClick={updateAll}
              disabled={applying}
              className="rounded-lg bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
            >
              {applying ? 'Updating…' : `Update all (${pending})`}
            </button>
          )}
        </div>
      </div>

      {found && (
        <ul className="space-y-1 text-sm">
          {found.app ? (
            <li className="flex justify-between gap-2">
              <span>Agora</span>
              <span className="text-muted-foreground">{found.app.currentVersion} → {found.app.version}</span>
            </li>
          ) : found.appError ? (
            <li className="text-xs text-destructive">
              Could not check for an Agora update: {found.appError}
            </li>
          ) : (
            <li className="text-xs text-muted-foreground">
              {found.appSkipped ?? 'Agora is up to date.'}
            </li>
          )}
          {found.plugins.map((row) => (
            <li key={row.id} className="flex flex-wrap justify-between gap-2">
              <span>{row.name}</span>
              <span className="text-muted-foreground">
                {row.from} → {row.to}
                {row.state === 'installing' && ' · updating…'}
                {row.state === 'installed' && ' · updated'}
                {(row.state === 'needs-consent' || row.state === 'failed') && ` · ${row.message}`}
              </span>
            </li>
          ))}
          {found.plugins.length === 0 && (
            <li className="text-xs text-muted-foreground">No plugin updates.</li>
          )}
          {found.pluginErrors.map((message) => (
            <li key={message} className="text-xs text-destructive">{message}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
