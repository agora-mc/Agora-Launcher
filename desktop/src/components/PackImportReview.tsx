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
  download: 'Download link in the pack',
  bundled: 'Bundled in the pack',
  cannot_get: 'Cannot be fetched',
};

/** "12 from Modrinth, 3 bundled in the pack": where the files come from, in one line. */
function sourceSummary(entries: PlannedPackEntry[]): string {
  const count = (source: PlannedPackSource) => entries.filter((entry) => entry.source === source).length;
  const parts: string[] = [];
  const add = (n: number, text: string) => { if (n > 0) parts.push(`${n} ${text}`); };
  add(count('curated'), 'from the catalog');
  add(count('modrinth'), 'from Modrinth');
  add(count('download'), 'from a download link in the pack');
  add(count('bundled'), 'bundled in the pack');
  add(count('cannot_get'), 'that cannot be fetched');
  return parts.join(', ');
}

/** Counts for the summary line, from the plan itself. */
function tally(entries: PlannedPackEntry[]) {
  const total = entries.length;
  const waiting = entries.filter((entry) => entry.action !== null).length;
  const blocked = entries.filter((entry) => entry.action === null && entry.reason !== null).length;
  const checkable = entries.filter((entry) => entry.checkable).length;
  return { total, waiting, blocked, checkable };
}

/**
 * The review step before a pack file (Agora pack or .mrpack) is imported: what
 * the instance will be, its name, and each file with where it comes from, what
 * is left out and what would stop the import. Nothing is written until Import.
 */
export function PackImportReview({
  label,
  plan,
  error,
  name,
  onNameChange,
  onImport,
  onCancel,
}: {
  /** The file name the import was started from. */
  label: string;
  /** Null while the plan is being read. */
  plan: PackImportPlan | null;
  error: string | null;
  /** The name the new instance will get. */
  name: string;
  onNameChange: (name: string) => void;
  onImport: () => void;
  onCancel: () => void;
}) {
  const counts = plan ? tally(plan.entries) : null;
  // Mods that will be downloaded: usable source, nothing already wrong with them.
  const downloadCount = plan
    ? plan.entries.filter((entry) => entry.source !== 'cannot_get' && entry.source !== 'bundled' && entry.reason === null).length
    : 0;
  const blocked = plan !== null && plan.problems.length > 0;
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
                downloaded. Couldn&apos;t connect: no internet, or the server can&apos;t be reached right
                now. Nothing was installed. Try again when you&apos;re online.
              </p>
            )}
            <div className="space-y-1">
              <label htmlFor="pack-import-name" className="block text-xs font-medium">
                Instance name
              </label>
              <input
                id="pack-import-name"
                value={name}
                onChange={(event) => onNameChange(event.target.value)}
                className="w-full rounded-lg border border-border bg-background px-3 py-2 text-sm"
              />
              {plan.name_taken && (
                <p className="text-xs text-muted-foreground">
                  An instance named &quot;{plan.pack_name}&quot; already exists, so this will be
                  imported as a separate copy. The existing instance is not changed.
                </p>
              )}
            </div>
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
              {plan.entries.length > 0 && (
                <>
                  <dt className="text-muted-foreground">From</dt>
                  <dd>{sourceSummary(plan.entries)}</dd>
                </>
              )}
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
                    {entry.arrives_disabled && <span className="text-muted-foreground"> · arrives disabled</span>}
                  </p>
                  {entry.reason && (
                    <p className={entry.action ? 'text-muted-foreground' : 'text-destructive'}>
                      {entry.reason}
                    </p>
                  )}
                </li>
              ))}
            </ul>
            {plan.left_out.length > 0 && (
              <div className="space-y-1">
                <p className="text-xs font-medium">Left out</p>
                <ul aria-label="Left out of the import" className="max-h-28 space-y-0.5 overflow-y-auto rounded-lg border border-border p-2 text-xs text-muted-foreground">
                  {plan.left_out.map((item, index) => <li key={`${index}-${item}`} className="break-all">{item}</li>)}
                </ul>
              </div>
            )}
            {plan.notes.map((note) => (
              <p key={note} className="text-xs text-muted-foreground">{note}</p>
            ))}
            {blocked && (
              <div role="alert" className="space-y-1 rounded-lg bg-destructive p-3 text-sm text-destructive-foreground">
                <p className="font-medium">Agora can&apos;t import this pack as it is:</p>
                <ul className="list-disc space-y-0.5 pl-4">
                  {plan.problems.map((problem) => <li key={problem} className="break-all">{problem}</li>)}
                </ul>
              </div>
            )}
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
            disabled={!plan || blocked || name.trim() === ''}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {plan?.name_taken ? 'Import as a copy' : 'Import'}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
