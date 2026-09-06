import { useState } from 'react';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import type { InstalledContentRow } from '@/lib/tauri';

/**
 * Pick or create the group a set of installed content belongs to.
 *
 * Grouping is the one arrangement of the mod list the user owns — pack,
 * category and source are all derived from the mods themselves. An item belongs
 * to at most one group, so this is a single choice rather than a checklist.
 */
export function ModGroupDialog({
  rows,
  groups,
  busy = false,
  onConfirm,
  onClose,
}: {
  rows: InstalledContentRow[];
  groups: string[];
  busy?: boolean;
  onConfirm: (group: string | null) => void;
  onClose: () => void;
}) {
  const [choice, setChoice] = useState<string>('');
  const [newName, setNewName] = useState('');

  const target = choice === '__new__' ? newName.trim() : choice === '' ? null : choice;
  const canConfirm = choice !== '__new__' || newName.trim().length > 0;

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="max-w-md">
        <DialogTitle>Set group</DialogTitle>
        <DialogDescription>
          {rows.length === 1
            ? rows[0].display_name
            : `${rows.length} selected items`}
        </DialogDescription>

        <div className="mt-4 space-y-3">
          <select
            value={choice}
            onChange={(e) => setChoice(e.target.value)}
            disabled={busy}
            className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
            aria-label="Group"
          >
            <option value="">Ungrouped</option>
            {groups.map((group) => (
              <option key={group} value={group}>{group}</option>
            ))}
            <option value="__new__">New group…</option>
          </select>

          {choice === '__new__' && (
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              disabled={busy}
              maxLength={64}
              placeholder="Group name"
              aria-label="New group name"
              className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
            />
          )}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="rounded-lg border border-input bg-background px-4 py-2 text-sm font-medium hover:bg-accent disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => onConfirm(target)}
            disabled={busy || !canConfirm}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy ? 'Saving…' : 'Save'}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
