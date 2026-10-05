/**
 * The "Launch failed" notice shown on an instance whose game exited abnormally.
 * Shared by the My Instances card and the instance editor so both say the same
 * thing in the same way.
 *
 * Colours use the readable red pair (`text-red-700` / `dark:text-red-300`)
 * rather than the raw `destructive` token, which is too dark to read against
 * the dark card surfaces. The actions wrap beneath the sentence on a narrow
 * card instead of squeezing the text into a thin column.
 */
export function LaunchFailedBanner({
  exitCode,
  onOpenConsole,
  onInvestigate,
  onDismiss,
}: {
  exitCode: number | null;
  onOpenConsole: () => void;
  onInvestigate: () => void;
  onDismiss?: () => void;
}) {
  const actionClass =
    'rounded border border-red-700/40 bg-background/60 px-2 py-1 text-xs font-medium text-foreground hover:bg-background dark:border-red-300/40';
  return (
    <div
      className="mt-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-2 rounded-lg border border-red-700/50 bg-red-500/10 px-3 py-2 text-xs dark:border-red-300/40"
      role="alert"
      aria-label="Launch failed"
    >
      <div className="min-w-[12rem] flex-1">
        <p className="font-semibold text-red-700 dark:text-red-300">Launch failed</p>
        <p className="text-foreground/80">
          The game exited with an error{exitCode != null ? ` (code ${exitCode})` : ''} before you could play.
        </p>
      </div>
      <div className="flex flex-wrap gap-2">
        <button type="button" onClick={onOpenConsole} className={actionClass}>
          Open Console
        </button>
        <button type="button" onClick={onInvestigate} className={actionClass}>
          Investigate
        </button>
        {onDismiss ? (
          <button type="button" onClick={onDismiss} className={actionClass}>
            Dismiss
          </button>
        ) : null}
      </div>
    </div>
  );
}
