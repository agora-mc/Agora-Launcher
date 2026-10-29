import { useEffect, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import { ChevronDown, ChevronRight } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog';
import { formatError, getUpdateChangelogs } from '@/lib/tauri';
import type { UpdateInfo, VersionChangelog } from '@/lib/tauri';

/** One pending update plus whatever the catalog knows about what changed. */
interface Entry {
  update: UpdateInfo;
  displayName: string;
  changelogs: VersionChangelog[] | null;
  error: string | null;
}

function ChangelogBody({ entries }: { entries: VersionChangelog[] }) {
  if (entries.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        No changelog is published for this update.
      </p>
    );
  }
  return (
    <div className="space-y-3">
      {entries.map((entry) => (
        <div key={`${entry.item_id}:${entry.version}`}>
          <div className="mb-1 flex items-baseline gap-2">
            <span className="text-xs font-semibold">{entry.version}</span>
            {entry.published_at ? (
              <span className="text-[10px] text-muted-foreground">
                {new Date(entry.published_at).toLocaleDateString()}
              </span>
            ) : null}
          </div>
          {/*
            react-markdown escapes raw HTML by default, so community-authored
            changelog text is rendered without dangerouslySetInnerHTML.
          */}
          <div className="prose prose-sm dark:prose-invert max-w-none text-xs">
            <ReactMarkdown>{entry.changelog}</ReactMarkdown>
          </div>
        </div>
      ))}
    </div>
  );
}

/**
 * Shows what changed before an update is applied.
 *
 * Changelogs come from the signed catalog, so this never blocks on the
 * network and an item with nothing published simply reads "no changelog"
 * rather than failing.
 */
export function UpdateChangelogDialog({
  updates,
  displayNameFor,
  onConfirm,
  onCancel,
}: {
  updates: UpdateInfo[];
  displayNameFor: (update: UpdateInfo) => string;
  /** Called with the updates left ticked; every one, unless the user unticked some. */
  onConfirm: (selected: UpdateInfo[]) => void;
  onCancel: () => void;
}) {
  const [skipped, setSkipped] = useState<Set<string>>(() => new Set());
  const [entries, setEntries] = useState<Entry[]>([]);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState<Set<string>>(
    // A single update shows its changelog immediately; a batch starts collapsed
    // so the list stays scannable.
    () => (updates.length === 1 ? new Set([updates[0].mod_jar_id]) : new Set()),
  );

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    void Promise.all(
      updates.map(async (update): Promise<Entry> => {
        const base: Entry = {
          update,
          displayName: displayNameFor(update),
          changelogs: null,
          error: null,
        };
        try {
          const changelogs = await getUpdateChangelogs(
            update.mod_jar_id,
            update.current_version,
            update.latest_version,
          );
          return { ...base, changelogs };
        } catch (error) {
          return { ...base, error: formatError(error) };
        }
      }),
    ).then((resolved) => {
      if (cancelled) return;
      setEntries(resolved);
      setLoading(false);
    });
    return () => { cancelled = true; };
    // `updates` is held in a stable state slot by the caller.
  }, [updates, displayNameFor]);

  const toggle = (key: string) => setExpanded((current) => {
    const next = new Set(current);
    if (next.has(key)) next.delete(key); else next.add(key);
    return next;
  });

  const single = updates.length === 1;
  const chosenCount = updates.filter((update) => !skipped.has(update.mod_jar_id)).length;

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onCancel(); }}>
      <DialogContent className="max-h-[80vh] max-w-2xl overflow-y-auto">
        <DialogTitle>
          {single ? `Update ${entries[0]?.displayName ?? ''}` : `Review ${updates.length} updates`}
        </DialogTitle>
        <DialogDescription>
          {single
            ? 'What changed since the version you have installed.'
            : "Ticked items are updated in one reviewed operation. Untick any to leave it as it is; to choose a different version, use that mod's page instead."}
        </DialogDescription>

        {loading ? (
          <p className="py-4 text-sm text-muted-foreground">Loading changelogs…</p>
        ) : (
          <div className="space-y-2">
            {entries.map((entry) => {
              const key = entry.update.mod_jar_id;
              const isOpen = expanded.has(key);
              return (
                <div key={key} className="rounded-lg border border-border p-3">
                  <div className="flex items-center gap-2">
                  {!single && (
                    <input
                      type="checkbox"
                      aria-label={`Update ${entry.displayName}`}
                      checked={!skipped.has(key)}
                      onChange={(e) => setSkipped((current) => {
                        const next = new Set(current);
                        if (e.target.checked) next.delete(key); else next.add(key);
                        return next;
                      })}
                      className="h-4 w-4 shrink-0 accent-primary"
                    />
                  )}
                  <button
                    type="button"
                    onClick={() => toggle(key)}
                    aria-expanded={isOpen}
                    className="flex w-full items-center gap-2 text-left text-sm"
                  >
                    {isOpen
                      ? <ChevronDown className="h-4 w-4 shrink-0" aria-hidden="true" />
                      : <ChevronRight className="h-4 w-4 shrink-0" aria-hidden="true" />}
                    <span className="font-medium">{entry.displayName}</span>
                    <span className="text-xs text-muted-foreground">
                      {entry.update.current_version} → {entry.update.latest_version}
                    </span>
                  </button>
                  </div>
                  {isOpen ? (
                    <div className="mt-2 border-t border-border pt-2">
                      {entry.error ? (
                        <p className="text-xs text-muted-foreground">
                          Could not load the changelog ({entry.error}). The update itself is unaffected.
                        </p>
                      ) : (
                        <ChangelogBody entries={entry.changelogs ?? []} />
                      )}
                    </div>
                  ) : null}
                </div>
              );
            })}
          </div>
        )}

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-lg border border-input bg-background px-3 py-2 text-sm hover:bg-accent"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => onConfirm(updates.filter((update) => !skipped.has(update.mod_jar_id)))}
            disabled={chosenCount === 0}
            className="rounded-lg bg-primary px-3 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {single ? 'Update' : chosenCount === updates.length ? `Update all ${updates.length}` : `Update ${chosenCount} of ${updates.length}`}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
