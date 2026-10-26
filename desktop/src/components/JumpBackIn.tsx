import { useEffect, useState } from 'react';
import { listRecentPlay, type QuickPlay, type RecentPlay } from '@/lib/tauri';

/**
 * The worlds and servers played most recently, one click from playing again.
 * Quick Play always launches directly, so it needs a Microsoft account.
 */
export function JumpBackIn({
  disabled,
  onPlay,
}: {
  disabled: boolean;
  onPlay: (instanceId: string, quickPlay: QuickPlay) => void;
}) {
  const [recent, setRecent] = useState<RecentPlay[]>([]);

  useEffect(() => {
    listRecentPlay(6)
      .then((found) => setRecent(Array.isArray(found) ? found : []))
      .catch(() => setRecent([]));
  }, []);

  if (recent.length === 0) return null;

  return (
    <section aria-labelledby="jump-back-in" className="space-y-2">
      <h2 id="jump-back-in" className="text-sm font-semibold">Jump back in</h2>
      <ul className="grid grid-cols-[repeat(auto-fill,minmax(14rem,1fr))] gap-2">
        {recent.map((entry) => (
          <li key={`${entry.kind}:${entry.instance_id}:${entry.target}`}>
            <button
              type="button"
              disabled={disabled}
              onClick={() => onPlay(entry.instance_id, { kind: entry.kind === 'server' ? 'multiplayer' : 'singleplayer', target: entry.target })}
              title={entry.kind === 'server' ? `Launch ${entry.instance_name} and join ${entry.target}` : `Launch ${entry.instance_name} and open ${entry.label}`}
              className="w-full rounded-lg border border-border bg-card px-3 py-2 text-left hover:bg-accent disabled:opacity-50"
            >
              <span className="block truncate text-sm font-medium">{entry.kind === 'server' ? '🌐 ' : '🗺 '}{entry.label}</span>
              <span className="block truncate text-xs text-muted-foreground">{entry.instance_name}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
