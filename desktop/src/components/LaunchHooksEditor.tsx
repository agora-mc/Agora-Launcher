import { useEffect, useState } from 'react';

/** Parse `NAME=VALUE` lines; blank lines and `#` comments are skipped. */
export function parseEnvLines(text: string): { env: Record<string, string>; error: string | null } {
  const env: Record<string, string> = {};
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const at = line.indexOf('=');
    if (at <= 0) return { env, error: `"${line}" is not NAME=VALUE.` };
    env[line.slice(0, at).trim()] = line.slice(at + 1);
  }
  return { env, error: null };
}

export function formatEnvLines(env: Record<string, string>): string {
  return Object.entries(env).map(([name, value]) => `${name}=${value}`).join('\n');
}

/**
 * Commands run before and after a direct launch, and extra environment
 * variables for the game.
 */
export function LaunchHooksEditor({
  preLaunch,
  postExit,
  env,
  disabled,
  onSave,
}: {
  preLaunch: string;
  postExit: string;
  env: Record<string, string>;
  disabled?: boolean;
  onSave: (preLaunch: string, postExit: string, env: Record<string, string>) => Promise<void>;
}) {
  const [pre, setPre] = useState(preLaunch);
  const [post, setPost] = useState(postExit);
  const [envText, setEnvText] = useState(formatEnvLines(env));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Keyed on the contents: callers rebuild `env` on every render, and
  // resetting on identity would wipe what is being typed.
  const envKey = formatEnvLines(env);
  useEffect(() => {
    setPre(preLaunch);
    setPost(postExit);
    setEnvText(envKey);
  }, [preLaunch, postExit, envKey]);

  const parsed = parseEnvLines(envText);

  const save = async () => {
    if (parsed.error) return;
    setSaving(true);
    setError(null);
    try {
      await onSave(pre.trim(), post.trim(), parsed.env);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const input = 'w-full rounded-lg border border-input bg-background px-3 py-2 text-sm font-mono';
  return (
    <div className="space-y-3">
      <label className="block space-y-1">
        <span className="text-sm font-medium">Pre-launch command</span>
        <span className="block text-xs text-muted-foreground">
          Runs in the instance folder before the game starts. If it fails, the game is not started.
        </span>
        <input aria-label="Pre-launch command" className={input} value={pre} disabled={disabled} onChange={(e) => setPre(e.target.value)} placeholder="python sync-config.py" />
      </label>
      <label className="block space-y-1">
        <span className="text-sm font-medium">Post-exit command</span>
        <span className="block text-xs text-muted-foreground">Runs after the game closes, for example to back something up.</span>
        <input aria-label="Post-exit command" className={input} value={post} disabled={disabled} onChange={(e) => setPost(e.target.value)} />
      </label>
      <p className="text-xs text-muted-foreground">
        Both get <code>INST_NAME</code>, <code>INST_ID</code>, <code>INST_DIR</code>, <code>INST_MC_DIR</code> and <code>INST_JAVA</code>,
        the same variables Prism Launcher provides. Arguments are split on spaces with quotes respected; no shell is used.
      </p>
      <label className="block space-y-1">
        <span className="text-sm font-medium">Environment variables</span>
        <span className="block text-xs text-muted-foreground">One <code>NAME=VALUE</code> per line, given to the game and to both commands.</span>
        <textarea aria-label="Environment variables" className={`${input} resize-y`} rows={3} value={envText} disabled={disabled} onChange={(e) => setEnvText(e.target.value)} placeholder="__GL_THREADED_OPTIMIZATIONS=1" />
      </label>
      {parsed.error && <p className="text-xs text-destructive">{parsed.error}</p>}
      {error && <p className="text-xs text-destructive">{error}</p>}
      <button
        type="button"
        onClick={() => void save()}
        disabled={disabled || saving || parsed.error !== null}
        className="rounded-lg border border-input px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50"
      >
        {saving ? 'Saving…' : 'Save launch commands'}
      </button>
    </div>
  );
}
