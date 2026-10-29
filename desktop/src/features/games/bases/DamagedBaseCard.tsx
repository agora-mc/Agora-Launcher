/**
 * A base record Agora cannot read, on the Setup tab. It cannot be checked, because checking needs the
 * record; it can be removed once no instance uses it.
 */

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import type { DamagedBaseSummary } from './api';
import { canRemove, DAMAGED_EXPLANATION, DAMAGED_TITLE, describeUsers } from './basesText';

interface DamagedBaseCardProps {
  base: DamagedBaseSummary;
  /** Asks for confirmation and removes the record. The section owns the refresh. */
  onRemove: () => void;
}

export function DamagedBaseCard({ base, onRemove }: DamagedBaseCardProps) {
  const removable = canRemove(base);

  return (
    <article className="space-y-3 rounded-xl border border-destructive/50 p-4" data-testid="damaged-base-card" aria-label={`Damaged base ${base.baseId}`}>
      <header className="flex flex-wrap items-center gap-2">
        <h3 className="text-sm font-semibold">{DAMAGED_TITLE}</h3>
        <Badge variant="destructive">Cannot be read</Badge>
      </header>

      <p className="text-sm">{DAMAGED_EXPLANATION}</p>

      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-muted-foreground">Record</dt>
        <dd className="break-all font-mono text-xs">{base.manifestPath}</dd>
        <dt className="text-muted-foreground">Problem</dt>
        <dd className="break-all text-xs">{base.error}</dd>
        <dt className="text-muted-foreground">Used by</dt>
        <dd>{describeUsers(base.usedBy)}</dd>
      </dl>

      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="destructive"
          size="sm"
          disabled={!removable}
          title={removable ? undefined : 'Remove or move the instances that use this base first.'}
          onClick={onRemove}
        >
          Remove base
        </Button>
      </div>
    </article>
  );
}
