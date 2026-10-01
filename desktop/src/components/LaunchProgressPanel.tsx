import { useEffect, useState } from 'react';
import {
  filesDetail,
  filesPercent,
  formatElapsed,
  type LaunchProgressInfo,
} from '@/lib/launchProgress';

/**
 * Compact launch status: stage, elapsed time, a bar while file counts are
 * known (indeterminate otherwise, none once ready) and a Console link.
 */
export function LaunchProgressPanel({
  progress,
  onOpenConsole,
  className = '',
}: {
  progress: LaunchProgressInfo;
  onOpenConsole?: () => void;
  className?: string;
}) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (progress.stage === 'ready') return undefined;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [progress.stage]);

  const percent = filesPercent(progress.files);
  const detail = filesDetail(progress.files);
  const showBar = progress.stage !== 'ready';

  return (
    <div
      className={`rounded-lg border border-border bg-muted/40 px-3 py-2 text-xs ${className}`}
      role="status"
      aria-label="Launch progress"
      data-stage={progress.stage}
    >
      <div className="flex items-center justify-between gap-3">
        <p className="min-w-0 truncate font-medium">
          {progress.label}
          {detail ? <span className="font-normal text-muted-foreground"> · {detail}</span> : null}
        </p>
        <div className="flex shrink-0 items-center gap-3 text-muted-foreground">
          {progress.stage !== 'ready' && (
            <span aria-label="Elapsed time">{formatElapsed(now - progress.startedAt)}</span>
          )}
          {onOpenConsole && (
            <button
              type="button"
              onClick={onOpenConsole}
              className="font-medium text-primary hover:underline"
            >
              Open Console
            </button>
          )}
        </div>
      </div>
      {showBar && (
        <div
          className="mt-2 h-1.5 overflow-hidden rounded-full bg-background"
          role="progressbar"
          aria-label={progress.label}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent ?? undefined}
        >
          <div
            className={`h-full rounded-full bg-primary transition-all duration-300 ${percent === null ? 'w-1/3 animate-pulse' : ''}`}
            style={percent === null ? undefined : { width: `${percent}%` }}
          />
        </div>
      )}
      {progress.stage === 'loading' && (
        <p className="mt-1 text-muted-foreground">
          The game process is up and still loading; Agora will say Ready once it reports that it has finished.
        </p>
      )}
    </div>
  );
}
