import { useEffect, useState } from 'react';
import { formatError, getProxySetting, restartApp, setProxySetting, type ProxySetting } from '@/lib/tauri';

/** How Agora's own requests reach the internet. Applied at startup. */
export function ProxySettings() {
  const [mode, setMode] = useState<ProxySetting['mode'] | null>(null);
  const [url, setUrl] = useState('');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [passwordStored, setPasswordStored] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getProxySetting()
      .then((view) => {
        const setting = view?.setting ?? { mode: 'system' };
        setMode(setting.mode);
        if (setting.mode === 'manual') {
          setUrl(setting.url);
          setUsername(setting.username ?? '');
        }
        setPasswordStored(Boolean(view?.password_stored));
      })
      .catch(() => setMode('system'));
  }, []);

  const save = async () => {
    setError(null);
    const setting: ProxySetting = mode === 'manual'
      ? { mode: 'manual', url: url.trim(), username: username.trim() || null }
      : { mode: mode ?? 'system' };
    try {
      // Leaving manual mode, or clearing the username, drops the stored password.
      const nextPassword = mode !== 'manual' || !username.trim() ? '' : password || undefined;
      await setProxySetting(setting, nextPassword);
      setPasswordStored(mode === 'manual' && Boolean(username.trim()) && (Boolean(password) || passwordStored));
      setPassword('');
      setSaved(true);
    } catch (e) {
      setError(formatError(e));
    }
  };

  if (mode === null) return <p className="text-xs text-muted-foreground">Loading proxy setting…</p>;

  const options: { value: ProxySetting['mode']; label: string }[] = [
    { value: 'system', label: 'Use the system environment (HTTPS_PROXY, HTTP_PROXY, ALL_PROXY)' },
    { value: 'none', label: 'No proxy' },
    { value: 'manual', label: 'Use this proxy' },
  ];
  const input = 'w-full rounded-lg border border-input bg-background px-3 py-1.5 text-sm';

  return (
    <section className="rounded-xl border border-border bg-card p-4 space-y-3" aria-labelledby="proxy-heading">
      <h2 id="proxy-heading" className="text-sm font-semibold">Proxy</h2>
      <p className="text-xs text-muted-foreground">
        For Agora&apos;s own downloads and sign-in. Minecraft itself does not use it. Changes apply after Agora restarts.
      </p>
      <div className="space-y-1.5" role="radiogroup" aria-label="Proxy">
        {options.map((option) => (
          <label key={option.value} className="flex items-center gap-2 text-sm">
            <input type="radio" name="proxy-mode" checked={mode === option.value} onChange={() => { setMode(option.value); setSaved(false); }} className="accent-primary" />
            {option.label}
          </label>
        ))}
      </div>
      {mode === 'manual' && (
        <div className="grid gap-2 pl-6 sm:grid-cols-3">
          <input aria-label="Proxy URL" className={`${input} sm:col-span-3 font-mono`} value={url} onChange={(e) => { setUrl(e.target.value); setSaved(false); }} placeholder="http://proxy.example:8080 or socks5://127.0.0.1:1080" />
          <input aria-label="Proxy username" className={input} value={username} onChange={(e) => { setUsername(e.target.value); setSaved(false); }} placeholder="Username (optional)" autoComplete="off" />
          <input aria-label="Proxy password" type="password" className={`${input} sm:col-span-2`} value={password} onChange={(e) => { setPassword(e.target.value); setSaved(false); }} placeholder={passwordStored ? 'Password stored; type to replace' : 'Password (optional)'} autoComplete="new-password" disabled={!username.trim()} />
          <p className="text-xs text-muted-foreground sm:col-span-3">The password is kept in your system&apos;s credential store, not in Agora&apos;s settings.</p>
        </div>
      )}
      {error && <p className="text-xs text-destructive">{error}</p>}
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" onClick={() => void save()} className="rounded-lg border border-input px-3 py-1.5 text-sm font-medium hover:bg-accent">
          Save proxy
        </button>
        {saved && (
          <>
            <span className="text-xs text-muted-foreground">Saved. Restart Agora to use it.</span>
            <button type="button" onClick={() => void restartApp()} className="rounded-lg bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90">
              Restart now
            </button>
          </>
        )}
      </div>
    </section>
  );
}
