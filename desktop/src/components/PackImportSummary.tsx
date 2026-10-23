import { useState } from 'react';
import { X } from 'lucide-react';
import {
  failedOutcomes,
  importHasIssues,
  importHeadline,
  needsActionOutcomes,
  offlineOutcomes,
  uncheckedOutcomes,
} from '../lib/packImport';
import type { PackImportReport } from '../lib/tauri';

/**
 * What an Agora pack import did, mod by mod. Shown whenever anything was not a
 * clean, checked install, so a partial import can never read as a success.
 */
export function PackImportSummary({
  report,
  onAllowModrinthOnce,
  onTurnOnModrinth,
  onRetry,
  onOpenSettings,
  onDismiss,
}: {
  report: PackImportReport;
  /** Retry the waiting Modrinth entries for this import only. */
  onAllowModrinthOnce: () => Promise<void>;
  /** Turn the Modrinth setting on for good, then retry. */
  onTurnOnModrinth: () => Promise<void>;
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
  const needsSettings = waiting.some((outcome) => outcome.action === 'open_settings');
  const offline = offlineOutcomes(report).length;
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
      className={`rounded-xl border bg-card p-4 shadow-sm ${issues ? 'border-amber-500/50' : 'border-border'}`}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold">{report.instance_name}</p>
          <p role="status" className="mt-1 text-sm">{importHeadline(report)}</p>
        </div>
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Dismiss import results"
          title="Dismiss import results"
          className="shrink-0 rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          <X className="h-4 w-4" aria-hidden="true" />
        </button>
      </div>

      {offline > 0 && (
        <p className="mt-2 text-sm">
          {offline === 1 ? 'This mod needs' : `These ${offline} mods need`} to be downloaded, and there&apos;s
          no internet connection right now.
          {report.installed === 0 ? ' Nothing was installed.' : ''} Try again when you&apos;re online.
        </p>
      )}

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

      {needsModrinth && (
        <p className="mt-3 text-xs text-muted-foreground">
          Allowing Modrinth sends requests for these mods to Modrinth. Modrinth stays off for everything else unless you turn it on.
        </p>
      )}

      {error && <p className="mt-2 text-xs text-destructive">{error}</p>}

      <div className="mt-3 flex flex-wrap justify-end gap-2">
        {needsModrinth && (
          <>
            <button
              type="button"
              disabled={busy}
              onClick={() => void run(onAllowModrinthOnce)}
              className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
            >
              Allow Modrinth for this import
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => void run(onTurnOnModrinth)}
              className="rounded-lg border border-input px-3 py-1.5 text-xs font-medium hover:bg-accent disabled:opacity-50"
            >
              Turn on Modrinth search (stays on, Settings &gt; Services)
            </button>
          </>
        )}
        {waiting.length > 0 && (
          <>
            {needsSettings && (
              <button
                type="button"
                onClick={onOpenSettings}
                className="rounded-lg border border-input px-3 py-1.5 text-xs font-medium hover:bg-accent"
              >
                Open Settings
              </button>
            )}
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
