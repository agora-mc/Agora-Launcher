import { useCallback, useEffect, useState } from 'react';
import {
  addServer,
  copyWorld,
  deleteWorld,
  exportWorld,
  formatError,
  importWorld,
  listServers,
  listWorlds,
  moveServer,
  openInstanceFolder,
  pingServer,
  removeServer,
  renameWorld,
  updateServer,
  type QuickPlay,
  type ServerEntry,
  type ServerStatus,
  type WorldInfo,
} from '@/lib/tauri';
import { useConfirm } from '@/components/ui/confirm';
import { formatBytes } from '@/components/installed-content/contentTableState';
import { formatDateTime } from '@/lib/utils';

const GAME_MODE_LABEL: Record<NonNullable<WorldInfo['game_mode']>, string> = {
  survival: 'Survival',
  creative: 'Creative',
  adventure: 'Adventure',
  spectator: 'Spectator',
};

const BUTTON = 'rounded-lg border border-input bg-background px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50';
const SMALL_BUTTON = 'rounded-md border border-input bg-background px-2 py-1 text-xs font-medium hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50';

/**
 * The instance's worlds and its in-game server list, with Quick Play.
 *
 * Every change is refused by the backend while the game is running, because
 * Minecraft rewrites these files on exit; the buttons are disabled then too so
 * the refusal is rarely needed.
 */
export function WorldsPanel({
  instanceId,
  running,
  sharing,
  onSetSharing,
  onQuickPlay,
  onStatus,
  onError,
}: {
  instanceId: string;
  running: boolean;
  /** Whether game settings and the server list are shared with other instances. */
  sharing?: { options: boolean; servers: boolean };
  onSetSharing?: (options: boolean, servers: boolean) => Promise<void>;
  onQuickPlay?: (quickPlay: QuickPlay) => Promise<boolean>;
  onStatus: (message: string) => void;
  onError: (message: string) => void;
}) {
  const { confirm, prompt } = useConfirm();
  const [worlds, setWorlds] = useState<WorldInfo[] | null>(null);
  const [servers, setServers] = useState<ServerEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  // Keyed by address; `null` while a ping is in flight. A refused ping
  // (Lockdown Mode, or the Server status switch off) is shown once, not per row.
  const [statuses, setStatuses] = useState<Record<string, ServerStatus | null>>({});
  const [pingBlocked, setPingBlocked] = useState<string | null>(null);

  const pingAll = useCallback(async (list: ServerEntry[]) => {
    const addresses = [...new Set(list.map((server) => server.address))];
    setStatuses(Object.fromEntries(addresses.map((address) => [address, null])));
    setPingBlocked(null);
    await Promise.all(addresses.map(async (address) => {
      try {
        const status = await pingServer(address);
        setStatuses((current) => ({ ...current, [address]: status ?? null }));
      } catch (e) {
        setPingBlocked(formatError(e));
        setStatuses((current) => {
          const next = { ...current };
          delete next[address];
          return next;
        });
      }
    }));
  }, []);

  const reload = useCallback(async () => {
    try {
      const [nextWorlds, nextServers] = await Promise.all([listWorlds(instanceId), listServers(instanceId)]);
      setWorlds(Array.isArray(nextWorlds) ? nextWorlds : []);
      setServers(Array.isArray(nextServers) ? nextServers : []);
      return Array.isArray(nextServers) ? nextServers : [];
    } catch (e) {
      onError(formatError(e));
      return [];
    }
  }, [instanceId, onError]);

  useEffect(() => {
    void reload().then((list) => { if (list.length > 0) void pingAll(list); });
  }, [reload, running, pingAll]);

  const run = async (action: () => Promise<string | null | void>) => {
    setBusy(true);
    try {
      const message = await action();
      if (message) onStatus(message);
      await reload();
    } catch (e) {
      onError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const play = (quickPlay: QuickPlay) => {
    if (!onQuickPlay) return;
    void run(async () => {
      await onQuickPlay(quickPlay);
    });
  };

  const editDisabled = busy || running;
  const editTitle = running ? 'Close the game before changing worlds or servers.' : undefined;

  const askServer = async (title: string, initial?: ServerEntry) => {
    const address = await prompt({
      title,
      body: 'Server address, for example play.example.net or play.example.net:25570.',
      initialValue: initial?.address ?? '',
      placeholder: 'play.example.net',
      confirmLabel: 'Next',
    });
    if (address === null || !address.trim()) return null;
    const name = await prompt({
      title: 'Server name',
      body: 'The name shown in the multiplayer list.',
      initialValue: initial?.name ?? '',
      placeholder: 'Minecraft Server',
      confirmLabel: 'Save',
    });
    if (name === null) return null;
    return { name, address };
  };

  return (
    <div className="space-y-8 p-1">
      {sharing && onSetSharing && (
        <section aria-labelledby="sharing-heading" className="rounded-lg border border-border p-3 space-y-2">
          <h2 id="sharing-heading" className="text-sm font-semibold">Share with other instances</h2>
          <p className="text-xs text-muted-foreground">
            Instances with these switched on keep the same settings: Agora writes the shared copy in before each launch and
            takes it back when the game closes. The first instance switched on provides the shared copy; later ones take it on
            at their next launch.
          </p>
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" className="h-4 w-4 accent-primary" disabled={busy || running} checked={sharing.options}
              onChange={(e) => void run(() => onSetSharing(e.target.checked, sharing.servers))} />
            Game settings: video, sound, controls and key bindings (options.txt; resource packs stay per instance)
          </label>
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" className="h-4 w-4 accent-primary" disabled={busy || running} checked={sharing.servers}
              onChange={(e) => void run(() => onSetSharing(sharing.options, e.target.checked))} />
            Server list
          </label>
        </section>
      )}
      <section aria-labelledby="worlds-heading" className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 id="worlds-heading" className="text-base font-semibold">Worlds</h2>
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              className={BUTTON}
              disabled={editDisabled}
              title={editTitle}
              onClick={() => void run(async () => {
                const folder = await importWorld(instanceId);
                return folder ? `Imported world as "${folder}".` : null;
              })}
            >
              Import world…
            </button>
            <button type="button" className={BUTTON} onClick={() => void openInstanceFolder(instanceId).catch((e) => onError(formatError(e)))}>
              Open instance folder
            </button>
          </div>
        </div>
        {onQuickPlay && (
          <p className="text-xs text-muted-foreground">
            Play opens the world straight from the launch (Minecraft 1.20 or newer) and always uses direct launch, so it needs a Microsoft account connected in Agora.
          </p>
        )}

        {worlds === null ? (
          <p className="text-sm text-muted-foreground">Loading worlds…</p>
        ) : worlds.length === 0 ? (
          <p className="rounded-lg border border-dashed border-border p-4 text-sm text-muted-foreground">
            No worlds yet. Create one in game, or import a world archive.
          </p>
        ) : (
          <ul className="divide-y divide-border rounded-lg border border-border">
            {worlds.map((world) => (
              <li key={world.folder} className="flex flex-wrap items-center gap-3 p-3" data-testid="world-row">
                {world.icon_data_url ? (
                  <img src={world.icon_data_url} alt="" className="h-12 w-12 shrink-0 rounded-md" style={{ imageRendering: 'pixelated' }} />
                ) : (
                  <div className="h-12 w-12 shrink-0 rounded-md bg-muted" aria-hidden="true" />
                )}
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium">{world.name}</p>
                  <p className="text-xs text-muted-foreground wrap-break-word">
                    {[
                      world.hardcore ? 'Hardcore' : world.game_mode ? GAME_MODE_LABEL[world.game_mode] : null,
                      world.version_name,
                      world.last_played_ms ? `Played ${formatDateTime(new Date(world.last_played_ms).toISOString())}` : null,
                      formatBytes(world.size_bytes),
                      world.folder !== world.name ? `Folder: ${world.folder}` : null,
                    ].filter(Boolean).join(' · ')}
                  </p>
                </div>
                <div className="flex flex-wrap gap-1.5">
                  {onQuickPlay && (
                    <button
                      type="button"
                      className="rounded-md bg-primary px-3 py-1 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
                      disabled={busy || running}
                      title={running ? 'The game is already running.' : `Launch and open ${world.name}`}
                      onClick={() => play({ kind: 'singleplayer', target: world.folder })}
                    >
                      Play
                    </button>
                  )}
                  <button
                    type="button"
                    className={SMALL_BUTTON}
                    disabled={editDisabled}
                    title={editTitle}
                    onClick={() => void run(async () => {
                      const name = await prompt({ title: 'Rename world', initialValue: world.name, confirmLabel: 'Rename' });
                      if (name === null || !name.trim() || name.trim() === world.name) return null;
                      await renameWorld(instanceId, world.folder, name);
                      return `Renamed to "${name.trim()}".`;
                    })}
                  >
                    Rename
                  </button>
                  <button
                    type="button"
                    className={SMALL_BUTTON}
                    disabled={editDisabled}
                    title={editTitle}
                    onClick={() => void run(async () => `Copied to "${await copyWorld(instanceId, world.folder)}".`)}
                  >
                    Copy
                  </button>
                  <button
                    type="button"
                    className={SMALL_BUTTON}
                    disabled={editDisabled}
                    title={editTitle}
                    onClick={() => void run(async () => {
                      const saved = await exportWorld(instanceId, world.folder);
                      return saved ? `Saved to ${saved}` : null;
                    })}
                  >
                    Export…
                  </button>
                  <button
                    type="button"
                    className={`${SMALL_BUTTON} text-destructive`}
                    disabled={editDisabled}
                    title={editTitle}
                    onClick={() => void run(async () => {
                      const ok = await confirm({
                        title: `Delete "${world.name}"?`,
                        body: 'The world moves to the system recycle bin, where you can still restore it. Snapshots taken before now also keep a copy.',
                        confirmLabel: 'Delete',
                        tone: 'danger',
                      });
                      if (!ok) return null;
                      await deleteWorld(instanceId, world.folder);
                      return `Moved "${world.name}" to the recycle bin.`;
                    })}
                  >
                    Delete
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section aria-labelledby="servers-heading" className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 id="servers-heading" className="text-base font-semibold">Servers</h2>
          <div className="flex flex-wrap gap-2">
          <button
            type="button"
            className={BUTTON}
            disabled={!servers || servers.length === 0}
            onClick={() => servers && void pingAll(servers)}
          >
            Refresh status
          </button>
          <button
            type="button"
            className={BUTTON}
            disabled={editDisabled}
            title={editTitle}
            onClick={() => void run(async () => {
              const entry = await askServer('Add server');
              if (!entry) return null;
              await addServer(instanceId, entry.name, entry.address);
              return 'Server added.';
            })}
          >
            Add server…
          </button>
          </div>
        </div>
        {pingBlocked && <p className="text-xs text-muted-foreground">Server status is off: {pingBlocked}</p>}
        <p className="text-xs text-muted-foreground">
          The same list as the game&apos;s Multiplayer screen.
        </p>
        {servers === null ? (
          <p className="text-sm text-muted-foreground">Loading servers…</p>
        ) : servers.length === 0 ? (
          <p className="rounded-lg border border-dashed border-border p-4 text-sm text-muted-foreground">No servers saved.</p>
        ) : (
          <ul className="divide-y divide-border rounded-lg border border-border">
            {servers.map((server, index) => (
              <li key={`${index}-${server.address}`} className="flex flex-wrap items-center gap-3 p-3" data-testid="server-row">
                {(statuses[server.address]?.icon_data_url ?? server.icon_data_url) ? (
                  <img src={statuses[server.address]?.icon_data_url ?? server.icon_data_url ?? undefined} alt="" className="h-10 w-10 shrink-0 rounded-md" style={{ imageRendering: 'pixelated' }} />
                ) : (
                  <div className="h-10 w-10 shrink-0 rounded-md bg-muted" aria-hidden="true" />
                )}
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium">{server.name || 'Minecraft Server'}</p>
                  <p className="truncate font-mono text-xs text-muted-foreground">{server.address}</p>
                  <ServerStatusLine status={statuses[server.address]} known={server.address in statuses} />
                </div>
                <div className="flex flex-wrap gap-1.5">
                  {onQuickPlay && (
                    <button
                      type="button"
                      className="rounded-md bg-primary px-3 py-1 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
                      disabled={busy || running}
                      title={running ? 'The game is already running.' : `Launch and join ${server.address}`}
                      onClick={() => play({ kind: 'multiplayer', target: server.address })}
                    >
                      Join
                    </button>
                  )}
                  <button type="button" className={SMALL_BUTTON} aria-label={`Move ${server.name} up`} disabled={editDisabled || index === 0} title={editTitle} onClick={() => void run(() => moveServer(instanceId, index, -1))}>↑</button>
                  <button type="button" className={SMALL_BUTTON} aria-label={`Move ${server.name} down`} disabled={editDisabled || index === servers.length - 1} title={editTitle} onClick={() => void run(() => moveServer(instanceId, index, 1))}>↓</button>
                  <button
                    type="button"
                    className={SMALL_BUTTON}
                    disabled={editDisabled}
                    title={editTitle}
                    onClick={() => void run(async () => {
                      const entry = await askServer('Edit server', server);
                      if (!entry) return null;
                      await updateServer(instanceId, index, entry.name, entry.address);
                      return 'Server updated.';
                    })}
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    className={`${SMALL_BUTTON} text-destructive`}
                    disabled={editDisabled}
                    title={editTitle}
                    onClick={() => void run(async () => {
                      const ok = await confirm({ title: `Remove "${server.name || server.address}"?`, confirmLabel: 'Remove', tone: 'danger' });
                      if (!ok) return null;
                      await removeServer(instanceId, index);
                      return 'Server removed.';
                    })}
                  >
                    Remove
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

function ServerStatusLine({ status, known }: { status: ServerStatus | null | undefined; known: boolean }) {
  if (!known) return null;
  if (status === null || status === undefined) {
    return <p className="text-xs text-muted-foreground">Checking…</p>;
  }
  if (!status.online) {
    return <p className="text-xs text-muted-foreground" title={status.error ?? undefined}>● Offline or unreachable</p>;
  }
  const parts = [
    status.players_online !== null && status.players_max !== null ? `${status.players_online}/${status.players_max} players` : null,
    status.latency_ms !== null ? `${status.latency_ms} ms` : null,
    status.version,
  ].filter(Boolean);
  return (
    <p className="text-xs wrap-break-word">
      <span className="text-green-600 dark:text-green-400">● Online</span>
      {parts.length > 0 && <span className="text-muted-foreground"> · {parts.join(' · ')}</span>}
      {status.motd && <span className="block truncate text-muted-foreground" title={status.motd}>{status.motd}</span>}
    </p>
  );
}
