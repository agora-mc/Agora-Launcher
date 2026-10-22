import { useState } from 'react';
import {
  failedOutcomes,
  importHasIssues,
  importHeadline,
  needsActionOutcomes,
  uncheckedOutcomes,
} from '../lib/packImport';
import type { PackImportReport } from '../lib/tauri';

/**
 * What an Agora pack import did, mod by mod. Shown whenever anything was not a
 * clean, checked install, so a partial import can never read as a success.
 */
export function PackImportSummary({
  report,
  onEnableModrinth,
  onRetry,
  onOpenSettings,
  onDismiss,
}: {
  report: PackImportReport;
  onEnableModrinth: () => Promise<void>;
  onRetry: () => Promise<void>;
  onOpenSettings: () => void;
  onDismiss: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const waiting = needsActionOutcomes(report);
  const failed = failedOutcomes(report);
  const unchecked = uncheckedOutcomes(report);
  const needsModrinth = waiting.some((outcome) => outcome.action === 'enable_modrinth');
  const issues = importHasIssues(report);

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section
      data-testid="pack-import-summary"
      aria-label={`Import of ${report.instance_name}`}
      className={`pointer-events-auto rounded-xl border bg-card/95 p-4 shadow-2xl backdrop-blur-sm ${issues ? 'border-amber-500/50' : 'border-border'}`}
    >
      <p className="text-sm font-semibold">{report.instance_name}</p>
      <p role="status" className="mt-1 text-sm">{importHeadline(report)}</p>

      <ul className="mt-2 max-h-48 space-y-2 overflow-y-auto text-xs">
        {waiting.map((outcome, index) => (
          <li key={`wait-${index}-${outcome.name}`}>
            <span className="font-medium" title={outcome.name}>{outcome.display_name || outcome.name}</span> needs action. {outcome.reason}
          </li>
        ))}
        {failed.map((outcome, index) => (
          <li key={`fail-${index}-${outcome.name}`} className="text-destructive">
            <span className="font-medium" title={outcome.name}>{outcome.display_name || outcome.name}</span> was not installed. {outcome.reason}
          </li>
        ))}
        {unchecked.map((outcome, index) => (
          <li key={`unchecked-${index}-${outcome.name}`} className="text-muted-foreground">
            <span className="font-medium" title={outcome.name}>{outcome.display_name || outcome.name}</span>: {outcome.reason}
          </li>
        ))}
        {report.warnings.map((warning, index) => (
          <li key={`${index}-${warning}`} className="text-muted-foreground">{warning}</li>
        ))}
      </ul>

      {error && <p className="mt-2 text-xs text-destructive">{error}</p>}

      <div className="mt-3 flex flex-wrap justify-end gap-2">
        {needsModrinth && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void run(onEnableModrinth)}
            className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            Turn on Modrinth for this
          </button>
        )}
        {waiting.length > 0 && (
          <>
            <button
              type="button"
              onClick={onOpenSettings}
              className="rounded-lg border border-input px-3 py-1.5 text-xs font-medium hover:bg-accent"
            >
              Open Settings
            </button>
            {!needsModrinth && (
              <button
                type="button"
                disabled={busy}
                onClick={() => void run(onRetry)}
                className="rounded-lg border border-input px-3 py-1.5 text-xs font-medium hover:bg-accent disabled:opacity-50"
              >
                Try again
              </button>
            )}
          </>
        )}
        <button
          type="button"
          onClick={onDismiss}
          className="rounded-lg border border-input px-3 py-1.5 text-xs font-medium hover:bg-accent"
        >
          Dismiss
        </button>
      </div>
    </section>
  );
}
