import { useMemo, useState } from 'react';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import type { OrphanedDependency } from '@/lib/tauri';

/**
 * Offer to clean up mods that were only ever installed as dependencies and that
 * nothing needs any more.
 *
 * Always a prompt, never automatic: the flag that makes this safe
 * (`installed_as_dependency`) is only as good as the install path that set it,
 * and legacy manifests have no provenance at all. The user is the check.
 */
export function OrphanCleanupDialog({
  orphans,
  busy = false,
  onConfirm,
  onClose,
}: {
  orphans: OrphanedDependency[];
  busy?: boolean;
  onConfirm: (filenames: string[]) => void;
  onClose: () => void;
}) {
  const [excluded, setExcluded] = useState<Record<string, boolean>>({});

  const selected = useMemo(
    () => orphans.filter((orphan) => !excluded[orphan.filename]).map((o) => o.filename),
    [orphans, excluded],
  );

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="max-w-lg">
        <DialogTitle>Remove unused dependencies?</DialogTitle>
        <DialogDescription>
          {orphans.length === 1
            ? 'This mod was installed as a dependency and nothing needs it any more.'
            : `These ${orphans.length} mods were installed as dependencies and nothing needs them any more.`}
        </DialogDescription>

        <div className="mt-4 max-h-72 space-y-1 overflow-y-auto">
          {orphans.map((orphan) => (
            <label key={orphan.filename} className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={!excluded[orphan.filename]}
                disabled={busy}
                onChange={(e) => setExcluded((prev) => ({
                  ...prev,
                  [orphan.filename]: !e.target.checked,
                }))}
              />
              <span className="truncate">{orphan.mod_jar_id ?? orphan.filename}</span>
              <span className="ml-auto shrink-0 text-xs text-muted-foreground">
                {orphan.filename}
              </span>
            </label>
          ))}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="rounded-lg border border-input bg-background px-4 py-2 text-sm font-medium hover:bg-accent disabled:opacity-50"
          >
            Keep them
          </button>
          <button
            type="button"
            onClick={() => onConfirm(selected)}
            disabled={busy || selected.length === 0}
            className="rounded-lg bg-destructive px-4 py-2 text-sm font-medium text-destructive-foreground hover:bg-destructive/90 disabled:opacity-50"
          >
            {busy ? 'Removing…' : `Remove ${selected.length}`}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
