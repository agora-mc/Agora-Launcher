/**
 * One pinned base on the Setup tab: what it is, who uses it, its checks, and its removal.
 */

import { useState } from 'react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { isGamesError } from '../shared/types';
import { verifyBase, type BaseSummary, type VerifyReply } from './api';
import {
  canRemove,
  describeProblem,
  describeUsers,
  diskText,
  gameWritesNote,
  linkedWarning,
  modeLabel,
  verifySummary,
} from './basesText';

interface BaseCardProps {
  base: BaseSummary;
  /** Asks for confirmation and removes the base. The section owns the refresh. */
  onRemove: () => void;
}

type Check = { status: 'idle' } | { status: 'running'; full: boolean } | { status: 'done'; reply: VerifyReply } | { status: 'failed'; message: string };

export function BaseCard({ base, onRemove }: BaseCardProps) {
  const [check, setCheck] = useState<Check>({ status: 'idle' });
  const removable = canRemove(base);

  async function runCheck(full: boolean) {
    setCheck({ status: 'running', full });
    try {
      const reply = await verifyBase(base.baseId, full);
      setCheck({ status: 'done', reply });
    } catch (error: unknown) {
      setCheck({
        status: 'failed',
        message: isGamesError(error) ? error.message : 'Agora could not check this base. Try again.',
      });
    }
  }

  const running = check.status === 'running';

  return (
    <article className="space-y-3 rounded-xl border border-border p-4" data-testid="base-card" aria-label={`${base.store} ${base.version} base`}>
      <header className="flex flex-wrap items-center gap-2">
        <h3 className="text-sm font-semibold">
          {base.store} {base.version}
          {base.build ? ` (build ${base.build})` : ''}
        </h3>
        <Badge variant="secondary">{modeLabel(base.mode)}</Badge>
        {!base.present && <Badge variant="destructive">Folder missing</Badge>}
      </header>

      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-muted-foreground">Disk</dt>
        <dd>{diskText(base)}</dd>
        <dt className="text-muted-foreground">Files</dt>
        <dd>
          {base.fileCount} files
          {base.skippedCount > 0 ? `, ${base.skippedCount} left out (links and tool backups)` : ''}
        </dd>
        <dt className="text-muted-foreground">Folder</dt>
        <dd className="break-all font-mono text-xs">{base.location}</dd>
        <dt className="text-muted-foreground">Used by</dt>
        <dd>{describeUsers(base.usedBy)}</dd>
      </dl>

      {!base.present && (
        <p role="alert" className="text-sm text-destructive">
          The base folder is missing. Instances on it cannot launch until the base is built again.
        </p>
      )}

      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" size="sm" disabled={running} onClick={() => runCheck(false)}>
          Check base (quick)
        </Button>
        <Button type="button" variant="outline" size="sm" disabled={running} onClick={() => runCheck(true)}>
          Check every file (full)
        </Button>
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
      <p className="text-xs text-muted-foreground">
        A quick check compares sizes and times and finishes at once. A full check reads every file again and can take
        several minutes on a large game.
      </p>

      {check.status === 'running' && (
        <p role="status" className="text-sm text-muted-foreground">
          {check.full ? 'Reading every file again…' : 'Checking sizes and times…'}
        </p>
      )}

      {check.status === 'failed' && (
        <p role="alert" className="text-sm text-destructive">{check.message}</p>
      )}

      {check.status === 'done' && <CheckResult reply={check.reply} />}
    </article>
  );
}

function CheckResult({ reply }: { reply: VerifyReply }) {
  const writes = gameWritesNote(reply.gameWrites);
  return (
    <div className="space-y-2 rounded-lg bg-muted/40 p-3 text-sm" data-testid="base-check-result">
      <p role="status" className={reply.clean ? 'font-medium' : 'font-medium text-destructive'}>
        {verifySummary(reply)}
      </p>
      {reply.problems.length > 0 && (
        <ul className="list-disc space-y-1 pl-5">
          {reply.problems.map((problem) => {
            const linked = linkedWarning(problem);
            return (
              <li key={`${problem.kind}:${problem.path}`}>
                <span className="font-mono text-xs">{problem.path}</span>: {describeProblem(problem)}
                {linked ? ` ${linked}` : ''}
              </li>
            );
          })}
        </ul>
      )}
      {writes && <p className="text-muted-foreground">{writes}</p>}
    </div>
  );
}
