import { useState } from 'react';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import type { InstalledContentRow } from '@/lib/tauri';

/**
 * Choose which worlds a data pack is copied into: every world (including ones
 * created later) or specific existing worlds.
 */
export function DatapackWorldsDialog({
  row,
  busy = false,
  onSave,
  onClose,
}: {
  row: InstalledContentRow;
  busy?: boolean;
  /** `null` means all worlds. */
  onSave: (worlds: string[] | null) => void;
  onClose: () => void;
}) {
  const status = row.world_sync;
  const available = status?.available_worlds ?? [];
  const [all, setAll] = useState(status?.all_worlds ?? true);
  const [selected, setSelected] = useState<Set<string>>(() => new Set(status?.selected_worlds ?? []));
  const toggle = (world: string) => setSelected((current) => {
    const next = new Set(current);
    if (next.has(world)) next.delete(world); else next.add(world);
    return next;
  });
  // Keep choices for worlds that no longer exist out of the saved list.
  const chosen = available.filter((world) => selected.has(world));

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="max-w-md">
        <DialogTitle>Worlds for this data pack</DialogTitle>
        <DialogDescription>{row.display_name}</DialogDescription>

        <div className="mt-4 space-y-3 text-sm">
          <label className="flex items-start gap-2">
            <input type="radio" name="datapack-scope" checked={all} onChange={() => setAll(true)} disabled={busy} />
            <span>All worlds<span className="block text-xs text-muted-foreground">Including worlds created later, from the next sync.</span></span>
          </label>
          <label className="flex items-start gap-2">
            <input type="radio" name="datapack-scope" checked={!all} onChange={() => setAll(false)} disabled={busy || available.length === 0} />
            <span>Only these worlds{available.length === 0 ? <span className="block text-xs text-muted-foreground">This instance has no worlds yet.</span> : null}</span>
          </label>
          {!all ? (
            <ul className="max-h-56 space-y-1 overflow-y-auto rounded-lg border border-border p-2" aria-label="Worlds">
              {available.map((world) => (
                <li key={world}>
                  <label className="flex items-center gap-2">
                    <input type="checkbox" checked={selected.has(world)} onChange={() => toggle(world)} disabled={busy} />
                    <span className="truncate" title={world}>{world}</span>
                  </label>
                </li>
              ))}
            </ul>
          ) : null}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <button type="button" onClick={onClose} disabled={busy} className="rounded-lg border border-input bg-background px-4 py-2 text-sm font-medium hover:bg-accent disabled:opacity-50">Cancel</button>
          <button
            type="button"
            onClick={() => onSave(all ? null : chosen)}
            disabled={busy || (!all && chosen.length === 0)}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy ? 'Saving…' : 'Save'}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
