import { useEffect, useState } from 'react';
import { getSetting, setSetting } from '@/lib/tauri';
import { usePlugins } from './PluginProvider';

const TOKENS: Record<string, string> = {
  background: 'background', foreground: 'background-foreground', surface: 'card',
  'surface-foreground': 'card-foreground', primary: 'primary', 'primary-foreground': 'primary-foreground',
  accent: 'accent', 'accent-foreground': 'accent-foreground', border: 'border', muted: 'muted',
  'muted-foreground': 'muted-foreground', destructive: 'destructive',
};

export function themeDeclarations(tokens: Record<string, string> | undefined): string {
  // A theme that defines only one mode is offered in that mode rather than
  // throwing while the app shell renders.
  return Object.entries(tokens ?? {}).flatMap(([token, value]) => {
    if (!TOKENS[token] || !/^#[\da-f]{6}$/i.test(value)) return [];
    const [r, g, b] = [1, 3, 5].map((offset) => parseInt(value.slice(offset, offset + 2), 16) / 255);
    const max = Math.max(r, g, b), min = Math.min(r, g, b), delta = max - min;
    const lightness = (max + min) / 2;
    const saturation = delta === 0 ? 0 : delta / (1 - Math.abs(2 * lightness - 1));
    let hue = delta === 0 ? 0 : max === r ? ((g - b) / delta) % 6 : max === g ? (b - r) / delta + 2 : (r - g) / delta + 4;
    hue = (hue * 60 + 360) % 360;
    return [`--${TOKENS[token]}:${hue.toFixed(2)} ${(saturation * 100).toFixed(2)}% ${(lightness * 100).toFixed(2)}% !important;`];
  }).join('');
}

export function PluginTheme() {
  const { plugins, ofKind } = usePlugins();
  const [selected, setSelected] = useState('');
  useEffect(() => {
    let mounted = true;
    const load = () => { void getSetting('plugin_theme').then((value) => {
      if (mounted) setSelected(typeof value === 'string' ? value : '');
    }).catch(() => { if (mounted) setSelected(''); }); };
    load();
    window.addEventListener('agora-plugin-theme', load);
    return () => { mounted = false; window.removeEventListener('agora-plugin-theme', load); };
  }, []);
  const contribution = ofKind('theme').find((entry) => entry.id === selected);
  const theme = plugins.find((plugin) => plugin.id === contribution?.pluginId)?.definitions?.theme;
  if (!theme) return null;
  return <style>{`:root:not(.dark){${themeDeclarations(theme.light)}}:root.dark{${themeDeclarations(theme.dark)}}`}</style>;
}

export function PluginThemeSelect() {
  const { ofKind } = usePlugins();
  const themes = ofKind('theme');
  const [selected, setSelected] = useState('');
  const [error, setError] = useState('');
  useEffect(() => { let mounted = true; void getSetting('plugin_theme').then((value) => {
    if (mounted) setSelected(typeof value === 'string' ? value : '');
  }).catch(() => {}); return () => { mounted = false; }; }, []);
  return <label className="block space-y-2 text-sm">Plugin theme
    <select aria-label="Plugin theme" className="ml-3 rounded border border-border bg-background p-2"
      value={selected} onChange={(event) => {
        const next = event.target.value;
        void setSetting('plugin_theme', next).then(() => {
          setSelected(next); setError(''); window.dispatchEvent(new Event('agora-plugin-theme'));
        }).catch(() => setError('The theme choice could not be saved.'));
      }}>
      <option value="">Use my built-in appearance</option>
      {selected && !themes.some((theme) => theme.id === selected) && <option value={selected}>Saved theme unavailable — using built-in appearance</option>}
      {themes.map((theme) => <option key={theme.id} value={theme.id}>{theme.title} ({theme.pluginId})</option>)}
    </select>
    {error && <span role="alert">{error}</span>}
  </label>;
}
