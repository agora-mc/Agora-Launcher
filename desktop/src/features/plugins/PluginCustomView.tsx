import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { formatError } from '@/lib/tauri';
import { usePlugins } from './PluginProvider';
import { runPluginCommand } from './api';

// A data document has an opaque origin. The sandbox grants scripts only: no
// forms, popups, downloads, top navigation, storage, or parent DOM access.
// Packages bundle their CSS/JS inline and images as data URLs for this spike.
export function customViewDocument(html: string): string {
  const policy = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
  return `<meta http-equiv="Content-Security-Policy" content="${policy}">${html}`;
}

export function PluginCustomView({ pluginId, localId, title, instanceId }: {
  pluginId: string; localId: string; title: string; instanceId?: string;
}) {
  const { plugins, enabled } = usePlugins();
  const frame = useRef<HTMLIFrameElement>(null);
  const [html, setHtml] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const commands = plugins.find((plugin) => plugin.id === pluginId)?.definitions?.commands;
  useEffect(() => {
    let cancelled = false;
    setHtml(null);
    setError(null);
    void invoke<string>('read_plugin_custom_view', { pluginId, localId })
      .then((value) => { if (!cancelled) setHtml(value); })
      .catch((reason: unknown) => { if (!cancelled) setError(formatError(reason)); });
    return () => { cancelled = true; };
  }, [pluginId, localId]);
  useEffect(() => {
    let disposed = false;
    let pending = false;
    let lastCall = 0;
    const receive = (event: MessageEvent) => {
      if (!enabled || event.source !== frame.current?.contentWindow || event.origin !== 'null') return;
      const request: unknown = event.data;
      if (!request || typeof request !== 'object') return;
      const { type, requestId, commandId, args } = request as Record<string, unknown>;
      if (type !== 'agora:command' || typeof requestId !== 'string' || requestId.length > 100 || typeof commandId !== 'string') return;
      const target = frame.current?.contentWindow;
      const reply = (payload: object) => {
        if (!disposed && target === frame.current?.contentWindow) target?.postMessage({ type: 'agora:result', requestId, ...payload }, '*');
      };
      const command = commands?.find((entry) => entry.id === commandId);
      if (!command) { reply({ error: 'Command is not declared by this plugin.' }); return; }
      if (pending || Date.now() - lastCall < 100) { reply({ error: 'Wait for the previous request before trying again.' }); return; }
      try {
        if (JSON.stringify(args ?? null).length > 65_536) throw new Error('Request too large');
      } catch { reply({ error: 'Request must be bounded JSON.' }); return; }
      pending = true;
      lastCall = Date.now();
      void runPluginCommand(pluginId, command.export, { ...(args && typeof args === 'object' ? args : {}), ...(instanceId ? { instanceId } : {}) })
        .then((value) => reply({ value }))
        .catch((reason: unknown) => reply({ error: formatError(reason) }))
        .finally(() => { pending = false; });
    };
    window.addEventListener('message', receive);
    return () => { disposed = true; window.removeEventListener('message', receive); };
  }, [pluginId, instanceId, enabled, commands]);
  if (error) return <p role="alert">{error}</p>;
  if (html === null) return <p role="status">Loading {title}…</p>;
  return <iframe ref={frame} title={title} sandbox="allow-scripts" referrerPolicy="no-referrer"
    className="min-h-[28rem] w-full rounded-lg border border-border"
    src={`data:text/html;charset=utf-8,${encodeURIComponent(customViewDocument(html))}`} />;
}
