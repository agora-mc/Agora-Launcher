import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog';
import type { PackImportPlan, PlannedPackEntry, PlannedPackSource } from '../lib/tauri';

const SOURCE_LABEL: Record<PlannedPackSource, string> = {
  curated: 'From the catalog',
  modrinth: 'From Modrinth',
  cannot_get: 'Cannot be fetched',
};

/** Counts for the summary line, from the plan itself. */
function tally(entries: PlannedPackEntry[]) {
  const total = entries.length;
  const waiting = entries.filter((entry) => entry.action !== null).length;
  const blocked = entries.filter((entry) => entry.action === null && entry.reason !== null).length;
  const checkable = entries.filter((entry) => entry.checkable).length;
  return { total, waiting, blocked, checkable };
}

/**
 * The review step before an Agora pack is imported: what the instance will be,
 * and each mod with where it comes from. Nothing is written until Import.
 */
export function PackImportReview({
  label,
  plan,
  error,
  onImport,
  onCancel,
}: {
  /** The file name the import was started from. */
  label: string;
  /** Null while the plan is being read. */
  plan: PackImportPlan | null;
  error: string | null;
  onImport: () => void;
  onCancel: () => void;
}) {
  const counts = plan ? tally(plan.entries) : null;
  // Mods that will be downloaded: usable source, nothing already wrong with them.
  const downloadCount = plan
    ? plan.entries.filter((entry) => entry.source !== 'cannot_get' && entry.reason === null).length
    : 0;
  // The webview's own view of the network. A hint, not proof, so the import still runs.
  const offlineNow = typeof navigator !== 'undefined' && navigator.onLine === false;

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onCancel(); }}>
      <DialogContent className="max-w-lg" data-testid="pack-import-review">
        <DialogTitle>{plan ? `Import ${plan.pack_name}?` : 'Review pack'}</DialogTitle>
        <DialogDescription>
          {plan
            ? 'Nothing is installed until you choose Import. Mods are fetched only when you do.'
            : `Reading ${label}…`}
        </DialogDescription>

        {error && (
          <p role="alert" className="rounded-lg bg-destructive p-3 text-sm text-destructive-foreground">
            {error}
          </p>
        )}

        {plan && counts && (
          <div className="space-y-3">
            {offlineNow && downloadCount > 0 && (
              <p role="alert" className="rounded-lg border border-amber-500/50 bg-amber-500/10 p-3 text-sm">
                {downloadCount === 1 ? 'This mod needs' : `These ${downloadCount} mods need`} to be
                downloaded, and there&apos;s no internet connection right now. Nothing was installed.
                Try again when you&apos;re online.
              </p>
            )}
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
              <dt className="text-muted-foreground">Minecraft</dt>
              <dd>{plan.instance.minecraft_version}</dd>
              <dt className="text-muted-foreground">Loader</dt>
              <dd>{plan.instance.loader} {plan.instance.loader_version}</dd>
              <dt className="text-muted-foreground">Mods</dt>
              <dd>
                {counts.total} in the pack
                {counts.waiting > 0 && `, ${counts.waiting} need an action first`}
                {counts.blocked > 0 && `, ${counts.blocked} will not install`}
                {`, ${counts.checkable} checked against the pack's hash`}
              </dd>
            </dl>

            <ul
              aria-label="Mods in this pack"
              className="max-h-72 space-y-2 overflow-y-auto rounded-lg border border-border p-2 text-xs"
            >
              {plan.entries.map((entry, index) => (
                <li key={`${index}-${entry.name}`} className="space-y-0.5">
                  <p>
                    <span className="font-medium" title={entry.name}>{entry.display_name || entry.name}</span>
                    <span className="text-muted-foreground"> · {SOURCE_LABEL[entry.source]}</span>
                    <span className="text-muted-foreground"> · {entry.checkable ? 'checked against the pack' : 'not checked against the pack'}</span>
                  </p>
                  {entry.reason && (
                    <p className={entry.action ? 'text-muted-foreground' : 'text-destructive'}>
                      {entry.reason}
                    </p>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onImport}
            disabled={!plan}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            Import
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
