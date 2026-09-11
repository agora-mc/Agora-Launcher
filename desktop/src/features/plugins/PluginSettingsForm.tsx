import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { showToast } from '@/components/Toast';
import { formatError } from '@/lib/tauri';
import { setPluginSetting } from './api';
import type { SettingDefinition } from './types';

/**
 * Renders a plugin's declared settings with Agora's own controls.
 *
 * The plugin does not draw this. It declares a type and a default in its
 * manifest, and the host renders and validates it — which is why the settings
 * page still works when the plugin's script is not loaded, and why a plugin
 * cannot store a value its own settings page would then fail to render.
 */

interface SettingsView {
  definitions: SettingDefinition[];
  values: Record<string, unknown>;
}

const getPluginSettings = (pluginId: string) =>
  invoke<SettingsView>('get_plugin_settings', { pluginId });

export function PluginSettingsForm({ pluginId }: { pluginId: string }) {
  const [view, setView] = useState<SettingsView | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const next = await getPluginSettings(pluginId);
        if (!cancelled) setView(next);
      } catch (e) {
        if (!cancelled) setError(formatError(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pluginId]);

  const write = useCallback(
    async (key: string, value: unknown) => {
      // Optimistic, then corrected: the backend validates against the declared
      // schema and rejects anything it does not like, so a refused write snaps
      // the control back rather than leaving the UI lying.
      setView((current) =>
        current ? { ...current, values: { ...current.values, [key]: value } } : current,
      );
      try {
        await setPluginSetting(pluginId, key, value);
      } catch (e) {
        showToast(formatError(e), 'error');
        try {
          setView(await getPluginSettings(pluginId));
        } catch {
          /* the toast already said what went wrong */
        }
      }
    },
    [pluginId],
  );

  if (error) return <p className="text-sm text-destructive">{error}</p>;
  if (!view) return <p className="text-sm text-muted-foreground">Loading settings…</p>;
  if (view.definitions.length === 0) {
    return <p className="text-sm text-muted-foreground">This plugin has no settings.</p>;
  }

  return (
    <div className="space-y-4">
      {view.definitions.map((definition) => (
        <SettingField
          key={definition.key}
          definition={definition}
          value={view.values[definition.key]}
          onChange={(value) => void write(definition.key, value)}
        />
      ))}
    </div>
  );
}

function SettingField({
  definition,
  value,
  onChange,
}: {
  definition: SettingDefinition;
  value: unknown;
  onChange: (value: unknown) => void;
}) {
  const id = `plugin-setting-${definition.key}`;

  const label = (
    <div className="min-w-0">
      <Label htmlFor={id}>{definition.title}</Label>
      {definition.description ? (
        <p className="text-xs text-muted-foreground">{definition.description}</p>
      ) : null}
    </div>
  );

  if (definition.type === 'boolean') {
    return (
      <div className="flex items-center justify-between gap-4">
        {label}
        <Switch id={id} checked={value === true} onCheckedChange={onChange} />
      </div>
    );
  }

  if (definition.type === 'enum') {
    return (
      <div className="space-y-1.5">
        {label}
        <select
          id={id}
          className="h-10 w-full rounded-md border border-input bg-background px-3 text-sm"
          value={typeof value === 'string' ? value : ''}
          onChange={(event) => onChange(event.target.value)}
        >
          {(definition.options ?? []).map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </div>
    );
  }

  if (definition.type === 'number') {
    return (
      <div className="space-y-1.5">
        {label}
        <Input
          id={id}
          type="number"
          min={definition.min}
          max={definition.max}
          value={typeof value === 'number' ? value : ''}
          onChange={(event) => {
            const next = Number(event.target.value);
            // An empty or unparseable box is not a number; leaving the stored
            // value alone beats writing NaN and having the backend reject it
            // on every keystroke.
            if (Number.isFinite(next)) onChange(next);
          }}
        />
      </div>
    );
  }

  return (
    <div className="space-y-1.5">
      {label}
      <Input
        id={id}
        type="text"
        maxLength={definition.maxLength}
        value={typeof value === 'string' ? value : ''}
        onChange={(event) => onChange(event.target.value)}
      />
    </div>
  );
}
