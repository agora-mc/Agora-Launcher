/**
 * The dry run of an import, shown before anything is stored: what would come in, what would be
 * skipped and why, and the button that makes it happen.
 */

import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { formatBytes } from '@/components/installed-content/contentTableState';
import type { Mo2PlanDto } from './api';
import { plannedModsLabel } from './planRules';

interface PlanViewProps {
  plan: Mo2PlanDto;
  /** Why the plan cannot be imported, or `null` when it can. Shown above the list. */
  blockReason: string | null;
  busy: boolean;
  onImport: () => void;
}

export function PlanView({ plan, blockReason, busy, onImport }: PlanViewProps) {
  return (
    <section
      aria-labelledby="import-plan-heading"
      className="space-y-4 rounded-xl border border-border bg-card p-5"
      data-testid="import-plan"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 id="import-plan-heading" className="text-base font-semibold">
            Preview: profile “{plan.profile}”
          </h3>
          <p className="mt-1 text-sm text-muted-foreground">
            Nothing has been stored yet. This is what an import would do.
          </p>
        </div>
        <Badge variant="secondary">{plan.mo2GameName}</Badge>
      </div>

      <dl className="grid gap-3 text-sm sm:grid-cols-3">
        <div>
          <dt className="text-muted-foreground">Mods</dt>
          <dd data-testid="import-plan-mods">{plannedModsLabel(plan)}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Files to store</dt>
          <dd>
            {plan.files} files, {formatBytes(plan.bytes)}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Store and install</dt>
          <dd className="break-all">
            {plan.store}: {plan.installLocation}
          </dd>
        </div>
      </dl>

      {blockReason && (
        <p role="alert" className="rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm">
          {blockReason}
        </p>
      )}

      {plan.mods.length > 0 && (
        <div>
          <h4 className="text-sm font-medium">Mods, highest priority first</h4>
          <ul className="mt-2 max-h-72 space-y-1 overflow-y-auto rounded-lg border border-border p-2 text-sm" aria-label="Mods in this profile">
            {plan.mods.map((mod) => (
              <li key={`${mod.line}-${mod.folder}`} className="flex flex-wrap justify-between gap-2">
                <span>
                  <span className="text-muted-foreground">line {mod.line}</span> {mod.folder}
                  {!mod.enabled && <span className="ml-2 text-muted-foreground">(disabled)</span>}
                </span>
                <span className="text-muted-foreground">
                  {mod.files} files, {formatBytes(mod.bytes)}. {mod.provenance}.
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}

      {plan.problems.length > 0 && (
        <div>
          <h4 className="text-sm font-medium">Skipped ({plan.problems.length})</h4>
          <p className="mt-1 text-sm text-muted-foreground">
            These lines are not imported. Nothing else is changed by skipping them.
          </p>
          <ul className="mt-2 space-y-1 text-sm" data-testid="import-plan-problems">
            {plan.problems.map((problem) => (
              <li key={`${problem.line ?? 'none'}-${problem.entry}`}>
                {problem.line !== null ? `line ${problem.line}: ` : ''}
                <span className="font-medium">{problem.entry}</span>: {problem.reason}
              </li>
            ))}
          </ul>
        </div>
      )}

      {plan.overwrite.present && (
        <div className="text-sm">
          <h4 className="font-medium">Overwrite folder</h4>
          <p className="mt-1 text-muted-foreground">
            {plan.overwrite.files} files, {formatBytes(plan.overwrite.bytes)}.
          </p>
          {plan.overwrite.generated.map((generated) => (
            <p key={generated.name} className="mt-1">
              {generated.files} file(s) from {generated.name} become its generated layer. Agora cannot tell what they
              were built from, so the launch check offers a rebuild instead of calling them current.
            </p>
          ))}
          {plan.overwrite.restFiles > 0 && (
            <p className="mt-1">{plan.overwrite.restFiles} other file(s) become the “MO2 overwrite” layer.</p>
          )}
        </div>
      )}

      <div className="grid gap-3 text-sm sm:grid-cols-2">
        <div>
          <h4 className="font-medium">Plugins</h4>
          <p className="mt-1 text-muted-foreground">
            {plan.plugins.kept
              ? `${plan.plugins.active} of ${plan.plugins.listed} plugins active, ${plan.plugins.locked.length} locked. Load order and locks come from MO2.`
              : 'This game keeps no plugin list, so none is imported.'}
          </p>
          {plan.plugins.notes.map((note) => (
            <p key={note} className="mt-1 text-muted-foreground">{note}</p>
          ))}
        </div>
        <div>
          <h4 className="font-medium">INIs and saves</h4>
          <p className="mt-1 text-muted-foreground">
            {plan.localSettings
              ? `The profile keeps its own INIs: ${plan.inis.join(', ') || 'none found'}.`
              : 'The game’s own INIs are used; the profile keeps none of its own.'}
          </p>
          <p className="mt-1 text-muted-foreground">
            {plan.saves.local
              ? `The profile keeps its own saves: ${plan.saves.files} file(s), ${formatBytes(plan.saves.bytes)}. ${plan.saves.note}`
              : 'The profile shares the game’s saves.'}
          </p>
        </div>
      </div>

      {plan.warnings.length > 0 && (
        <ul className="space-y-1 text-sm" aria-label="Warnings">
          {plan.warnings.map((warning) => (
            <li key={warning} className="text-amber-700 dark:text-amber-300">
              {warning}
            </li>
          ))}
        </ul>
      )}

      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border pt-4">
        <p className="text-sm text-muted-foreground">
          The setup at {plan.setup} is only read. It is not changed.
        </p>
        <Button onClick={onImport} disabled={busy || blockReason !== null}>
          Import as a new instance
        </Button>
      </div>
    </section>
  );
}
