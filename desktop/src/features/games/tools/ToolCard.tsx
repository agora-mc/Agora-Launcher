/**
 * One declared tool in an instance: its output, and the actions on it. Run asks how to capture the
 * output, and asks before anything slower or larger than the default happens (a refused virtual file
 * system, a tool that works on the real install, or a full-copy swap, whose costs are shown first).
 */

import { useState } from 'react';
import { useConfirm } from '@/components/ui/confirm';
import {
  diffGameTool,
  gameToolSwapCosts,
  removeGameTool,
  rollbackGameTool,
  runGameTool,
  type CaptureChoice,
  type DiffReport,
  type RunReport,
  type RunResult,
  type SwapCosts,
  type ToolOffer,
  type ToolRow,
  type ToolsView,
} from './api';
import {
  CAPTURE_CHOICES,
  messageOf,
  runDetailLines,
  runHeadline,
  shownPaths,
  statusExplanation,
  statusLabel,
  swapCostLines,
} from './toolCopy';

/** A question the run has put to the person. Nothing runs until they answer it. */
type Question =
  | { kind: 'refused'; reason: string; next: 'links' | 'swap' | null }
  | {
      kind: 'swap';
      /** The refusal the swap follows, when it follows one. */
      refusal: string | null;
      /** Why the swap is being offered, in a sentence. */
      why: string;
      /** Its costs, once they are shown. The swap cannot be confirmed before this. */
      costs: SwapCosts | null;
      costsError: string | null;
    };

interface ToolCardProps {
  instanceId: string;
  row: ToolRow;
  busy: boolean;
  setBusy: (busy: boolean) => void;
  onView: (view: ToolsView) => void;
  onChanged: () => void;
}

const SWAP_WHY_CHOSEN = 'You chose the full-copy swap.';
const SWAP_WHY_REAL_INSTALL = 'This tool works on the game’s real install folder, which the other captures would miss.';

export function ToolCard({ instanceId, row, busy, setBusy, onView, onChanged }: ToolCardProps) {
  const { confirm } = useConfirm();
  const [capture, setCapture] = useState<CaptureChoice>('auto');
  const [question, setQuestion] = useState<Question | null>(null);
  const [outcome, setOutcome] = useState<RunReport | null>(null);
  const [offers, setOffers] = useState<ToolOffer[]>([]);
  const [actionError, setActionError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [diff, setDiff] = useState<DiffReport | null>(null);

  const runLabel = row.status === null ? `Build ${row.name}` : row.status === 'current' ? `Run ${row.name} again` : `Rebuild ${row.name}`;
  const helpText = CAPTURE_CHOICES.find((choice) => choice.value === capture)?.help ?? '';

  const openSwap = (refusal: string | null, why: string, costs: SwapCosts | null = null) => {
    setQuestion({ kind: 'swap', refusal, why, costs, costsError: null });
    if (costs) return;
    gameToolSwapCosts(instanceId)
      .then((loaded) => setQuestion((current) => (current?.kind === 'swap' && current.why === why && current.refusal === refusal
        ? { ...current, costs: loaded }
        : current)))
      .catch((error: unknown) => setQuestion((current) => (current?.kind === 'swap' && current.why === why && current.refusal === refusal
        ? { ...current, costsError: messageOf(error) }
        : current)));
  };

  /** Show a run's answer: a finished run's result and offers, or a question to put to the person. */
  const handle = (result: RunResult) => {
    if (result.status === 'finished') {
      setQuestion(null);
      setOutcome(result.outcome);
      setOffers(result.outcome.offers);
      setNote(null);
      onChanged();
      return;
    }
    if (result.status === 'vfsRefused') {
      setOutcome(null);
      setQuestion({ kind: 'refused', reason: result.reason, next: result.next });
      setOffers(result.offers);
      return;
    }
    setOutcome(null);
    openSwap(null, SWAP_WHY_REAL_INSTALL);
  };

  const run = async (choice: CaptureChoice, options: { refusal?: string; confirmSwap?: boolean } = {}) => {
    setBusy(true);
    setActionError(null);
    setNote(null);
    try {
      handle(await runGameTool(instanceId, row.toolId, choice, options));
    } catch (error) {
      setActionError(messageOf(error));
    } finally {
      setBusy(false);
    }
  };

  const startRun = () => {
    setActionError(null);
    if (capture === 'swap') {
      // A swap is never run on the first click: its costs are shown, and the person confirms them.
      setOutcome(null);
      openSwap(null, SWAP_WHY_CHOSEN);
      return;
    }
    void run(capture);
  };

  const confirmSwap = (current: Question & { kind: 'swap' }) => {
    if (!current.costs) return;
    void run('swap', { refusal: current.refusal ?? undefined, confirmSwap: true });
  };

  const rollBack = async () => {
    setBusy(true);
    setActionError(null);
    try {
      const view = await rollbackGameTool(instanceId, row.toolId);
      setNote(`Rolled back. ${row.name}'s output is now generation ${view.tools.find((t) => t.toolId === row.toolId)?.current ?? '?'}, and the game reads it on its next launch.`);
      onView(view);
    } catch (error) {
      setActionError(messageOf(error));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    const ok = await confirm({
      title: `Remove ${row.name}’s output?`,
      body: 'Its generations are deleted from this instance. Your mods and the game’s own files are not touched. You can build it again later.',
      confirmLabel: 'Remove output',
      tone: 'danger',
    });
    if (!ok) return;
    setBusy(true);
    setActionError(null);
    try {
      const reply = await removeGameTool(instanceId, row.toolId);
      setNote(`Removed ${row.name}, with ${reply.removedGenerations.length} generation folder(s).`);
      setOutcome(null);
      setDiff(null);
      onView(reply.view);
    } catch (error) {
      setActionError(messageOf(error));
    } finally {
      setBusy(false);
    }
  };

  const showChanges = async () => {
    setBusy(true);
    setActionError(null);
    try {
      setDiff(await diffGameTool(instanceId, row.toolId));
    } catch (error) {
      setActionError(messageOf(error));
    } finally {
      setBusy(false);
    }
  };

  const generationLine = row.current === null
    ? 'No output here yet.'
    : `Output: generation ${row.current}${row.previous ? `, with generation ${row.previous} kept to roll back to` : ''}.`;

  return (
    <article
      aria-label={row.name}
      className="space-y-4 rounded-xl border border-border bg-card p-5"
      data-testid="tool-card"
      data-tool={row.toolId}
    >
      <header className="flex flex-wrap items-center gap-2">
        <h3 className="text-base font-semibold">{row.name}</h3>
        <span className="rounded-full border border-border px-2.5 py-0.5 text-xs font-semibold" data-testid="tool-status">
          {statusLabel(row.status)}
        </span>
        {!row.declared && (
          <span className="rounded-full border border-border px-2.5 py-0.5 text-xs text-muted-foreground">
            No longer declared by the game
          </span>
        )}
      </header>
      <p className="text-sm text-muted-foreground">{statusExplanation(row)}</p>
      <p className="text-sm">{generationLine}</p>

      <div className="flex flex-wrap items-end gap-3">
        <label className="flex flex-col gap-1 text-sm">
          <span className="font-medium">How to capture the output</span>
          <select
            value={capture}
            disabled={busy}
            onChange={(event) => setCapture(event.target.value as CaptureChoice)}
            className="rounded-lg border border-border bg-background px-2 py-1.5"
            aria-describedby={`tool-capture-help-${row.toolId}`}
          >
            {CAPTURE_CHOICES.map((choice) => (
              <option key={choice.value} value={choice.value}>{choice.label}</option>
            ))}
          </select>
          <span id={`tool-capture-help-${row.toolId}`} className="max-w-md text-xs text-muted-foreground">{helpText}</span>
        </label>
        <button
          type="button"
          disabled={busy}
          onClick={startRun}
          className="rounded-lg bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground shadow-sm disabled:opacity-50"
        >
          {runLabel}
        </button>
        {row.previous && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void rollBack()}
            className="rounded-lg border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50"
          >
            Roll back
          </button>
        )}
        {row.previous && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void showChanges()}
            className="rounded-lg border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50"
          >
            Show changes
          </button>
        )}
        {row.current !== null && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void remove()}
            className="rounded-lg border border-destructive px-3 py-1.5 text-sm font-medium text-destructive hover:bg-destructive/10 disabled:opacity-50"
          >
            Remove output
          </button>
        )}
      </div>

      {actionError && (
        <p role="alert" className="rounded-lg border border-destructive bg-destructive/10 p-3 text-sm">{actionError}</p>
      )}
      {note && <p role="status" className="text-sm text-muted-foreground">{note}</p>}

      {question?.kind === 'refused' && (
        <section role="region" aria-label="The tool could not run its usual way" className="space-y-3 rounded-lg border border-border p-4 text-sm" data-testid="tool-refused">
          <p className="font-medium">The game’s file layer could not start, so nothing ran.</p>
          <p className="text-muted-foreground">{question.reason}</p>
          <div className="flex flex-wrap gap-2">
            {question.next === 'links' && (
              <button
                type="button"
                disabled={busy}
                onClick={() => void run('links', { refusal: question.reason })}
                className="rounded-lg bg-primary px-3 py-1.5 font-medium text-primary-foreground shadow-sm disabled:opacity-50"
              >
                Run with link capture
              </button>
            )}
            {question.next === 'swap' && (
              <button
                type="button"
                disabled={busy}
                onClick={() => openSwap(question.reason, 'The file layer could not start, so the next option is the full-copy swap.')}
                className="rounded-lg bg-primary px-3 py-1.5 font-medium text-primary-foreground shadow-sm disabled:opacity-50"
              >
                Show the full-copy swap
              </button>
            )}
            <button
              type="button"
              onClick={() => setQuestion(null)}
              className="rounded-lg border border-border px-3 py-1.5 font-medium hover:bg-accent"
            >
              Not now
            </button>
          </div>
        </section>
      )}

      {question?.kind === 'swap' && (
        <section role="region" aria-label="Full-copy swap" className="space-y-3 rounded-lg border border-border p-4 text-sm" data-testid="tool-swap-confirm">
          <p className="font-medium">{question.why}</p>
          {question.costsError && <p role="alert" className="text-destructive">The swap’s costs could not be read: {question.costsError}</p>}
          {!question.costs && !question.costsError && <p className="text-muted-foreground">Working out what the swap would cost…</p>}
          {question.costs && (
            <ul className="list-disc space-y-1 pl-5" data-testid="tool-swap-costs">
              {swapCostLines(row.name, question.costs).map((line) => <li key={line}>{line}</li>)}
            </ul>
          )}
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              disabled={busy || !question.costs}
              onClick={() => confirmSwap(question)}
              className="rounded-lg bg-primary px-3 py-1.5 font-medium text-primary-foreground shadow-sm disabled:opacity-50"
            >
              Run by full-copy swap
            </button>
            <button
              type="button"
              onClick={() => setQuestion(null)}
              className="rounded-lg border border-border px-3 py-1.5 font-medium hover:bg-accent"
            >
              Cancel
            </button>
          </div>
        </section>
      )}

      {outcome && (
        <section role="status" aria-label="Last run" className="space-y-2 rounded-lg border border-border p-4 text-sm" data-testid="tool-outcome">
          <p className="font-medium">{runHeadline(outcome)}</p>
          <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
            {runDetailLines(outcome).map((line) => <li key={line}>{line}</li>)}
          </ul>
        </section>
      )}

      {offers.length > 0 && (
        <section aria-label="Things to try" className="space-y-2 rounded-lg border border-border p-4 text-sm" data-testid="tool-offers">
          <p className="font-medium">{row.name} has failed more than once in a row. You can try:</p>
          {offers.map((offer) => offer.offer === 'copySwap' ? (
            <div key="copy-swap" className="space-y-2">
              <p>The full-copy swap. It is slower and uses disk space, and it is never run without your say-so.</p>
              <button
                type="button"
                disabled={busy}
                onClick={() => openSwap(null, 'You chose to try the full-copy swap after repeated failures.', offer.costs)}
                className="rounded-lg border border-border px-3 py-1.5 font-medium hover:bg-accent disabled:opacity-50"
              >
                Show the full-copy swap
              </button>
            </div>
          ) : (
            <p key="switch-links">
              Linked files. Switch this instance to linked files from its Overview page if you want to try that.
            </p>
          ))}
        </section>
      )}

      {diff && (
        <section aria-label="Changes against the previous generation" className="space-y-2 text-sm" data-testid="tool-diff">
          <p className="font-medium">
            Generation {diff.current} compared with generation {diff.previous}
          </p>
          {diff.added.length + diff.changed.length + diff.removed.length === 0 && (
            <p className="text-muted-foreground">No file differs between the two generations.</p>
          )}
          {[
            { label: 'Added', paths: diff.added },
            { label: 'Changed', paths: diff.changed },
            { label: 'Removed', paths: diff.removed },
          ].map(({ label, paths }) => {
            if (paths.length === 0) return null;
            const { shown, more } = shownPaths(paths);
            return (
              <div key={label}>
                <p className="font-medium">{label} ({paths.length})</p>
                <ul className="list-disc pl-5 text-muted-foreground">
                  {shown.map((path) => <li key={path}>{path}</li>)}
                  {more > 0 && <li>… and {more} more</li>}
                </ul>
              </div>
            );
          })}
        </section>
      )}
    </article>
  );
}
