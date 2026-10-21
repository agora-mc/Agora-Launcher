import { useEffect, useState } from 'react';
import { Sparkles } from 'lucide-react';
import { SettingsSection } from './SettingsSection';
import { bundledCliPath, type BundledCli } from '../../lib/tauri';
import { agoraRepositoryUrl } from '../../lib/brandConfig';

/**
 * The message a person pastes into their assistant app. It names the CLI by
 * full path so it works in any folder, with no PATH setup, and asks for
 * `agora skill` — the guide that tells the agent to ask before every change.
 */
export function assistantMessage(cliPath: string): string {
  return `Agora's command-line tool is installed at "${cliPath}". Run it with the argument "skill" `
    + 'and follow the guide it prints. Then help me with: ';
}

/**
 * Hands the bundled CLI to an AI assistant app. Shown to everyone, unlike the
 * MCP controls below it: this is the path the documentation recommends, and
 * it needs no configuration.
 */
export function AiAssistantCard() {
  // undefined while loading; null when this build has no bundled CLI.
  const [cli, setCli] = useState<BundledCli | null | undefined>(undefined);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    bundledCliPath()
      .then((found) => { if (!cancelled) setCli(found); })
      .catch(() => { if (!cancelled) setCli(null); });
    return () => { cancelled = true; };
  }, []);

  const copy = async (path: string) => {
    await navigator.clipboard.writeText(assistantMessage(path));
    setCopied(true);
    window.setTimeout(() => setCopied(false), 2000);
  };

  return (
    <SettingsSection
      icon={Sparkles}
      title="Use with an AI assistant"
      description="Let an assistant app on your computer read crash logs and fix problems through Agora. It asks before it changes anything."
      contentClassName="space-y-3"
      data-testid="ai-assistant-card"
    >
      <ol className="list-decimal space-y-1.5 pl-5 text-sm text-muted-foreground">
        <li>
          Install an assistant app that can run commands, such as{' '}
          <a href="https://opencode.ai/" target="_blank" rel="noopener noreferrer" className="underline">OpenCode</a>{' '}
          (open source, with some free models), Codex in the ChatGPT app, Claude Code, or Google Antigravity.
        </li>
        <li>Copy the message below, paste it into the app, and finish it with what you need help with.</li>
        <li>Allow its commands one at a time. Keep the app&apos;s approval prompts on.</li>
      </ol>

      {cli === undefined && <p className="text-xs text-muted-foreground">Looking for the command-line tool…</p>}

      {cli && (
        <div className="space-y-2 rounded-lg bg-muted p-3">
          <p className="text-xs text-muted-foreground">Command-line tool</p>
          <code className="block break-all text-xs" data-testid="ai-assistant-cli-path">{cli.path}</code>
          {cli.temporary && (
            <p className="text-xs text-amber-600 dark:text-amber-400">
              This copy of Agora runs as an AppImage, so this path only works while Agora is open.
            </p>
          )}
          <button
            type="button"
            onClick={() => void copy(cli.path)}
            disabled={copied}
            className="rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {copied ? 'Copied!' : 'Copy message'}
          </button>
        </div>
      )}

      {cli === null && (
        <p className="text-xs text-muted-foreground">
          This build of Agora does not include the command-line tool. Download the{' '}
          <code>agora-cli</code> archive for your system from the{' '}
          <a href={`${agoraRepositoryUrl}/releases/latest`} target="_blank" rel="noopener noreferrer" className="underline">
            latest release
          </a>
          , unzip it, and tell your assistant where it is.
        </p>
      )}

      <p className="text-xs text-muted-foreground">
        Anything the assistant reads, such as crash logs, is sent to the company that runs it.
      </p>
    </SettingsSection>
  );
}
