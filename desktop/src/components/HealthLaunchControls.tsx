/**
 * The health notice and the "review before launch" actions for an instance
 * that has problems. Shared by the My Instances card and the Home "Continue"
 * card so both show the same counts, wording and buttons.
 */
import { Play } from 'lucide-react';
import { healthCounts, healthHeadline, healthLaunchNote } from '@/lib/healthSummary';
import type { HealthReport } from '../lib/tauri';

/**
 * Banner for a report with problems or warnings. Renders nothing when the report
 * is clean. `className` adds spacing around it, for the caller's layout.
 */
export function HealthNotice({ report, onReview, className = '' }: {
  report: HealthReport;
  onReview: () => void;
  className?: string;
}) {
  const counts = healthCounts(report);
  const healthIssueCount = counts.problems + counts.warnings;
  if (healthIssueCount === 0) return null;
  const healthBlocked = counts.problems > 0;

  return (
    <div
      className={`mt-3 flex items-center justify-between gap-3 rounded-lg border px-3 py-2 text-xs ${
        healthBlocked
          ? 'border-destructive/60 bg-destructive/10'
          : 'border-amber-500/60 bg-amber-500/10'
      } ${className}`}
      role="alert"
      aria-label={healthHeadline(counts)}
    >
      <div className="min-w-0">
        <p className={healthBlocked ? 'font-medium text-destructive' : 'font-medium text-amber-700 dark:text-amber-300'}>
          {healthHeadline(counts)}
        </p>
        <p className="truncate text-muted-foreground">
          {healthLaunchNote(counts)}
        </p>
      </div>
      <button
        type="button"
        onClick={onReview}
        className="shrink-0 rounded-sm border border-current/30 px-2 py-1 font-medium hover:bg-background/40"
      >
        Review & repair
      </button>
    </div>
  );
}

/**
 * The actions for an instance that is not ready: the main action is to see the
 * problems, and launch stays reachable as the secondary action (the health
 * dialog asks first).
 */
export function ReviewBeforeLaunchButtons({ onReview, onLaunch, disabled, launchLabel }: {
  onReview: () => void;
  onLaunch: () => void;
  disabled: boolean;
  launchLabel: string;
}) {
  return (
    <>
      <button
        onClick={onReview}
        disabled={disabled}
        className="inline-flex items-center gap-1.5 rounded-lg bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
      >
        See what&apos;s wrong
      </button>
      <button
        onClick={onLaunch}
        disabled={disabled}
        className="inline-flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50"
      >
        <Play className="h-3.5 w-3.5 fill-current" aria-hidden="true" />
        {launchLabel}
      </button>
    </>
  );
}
