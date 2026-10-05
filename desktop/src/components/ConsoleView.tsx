import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cn } from '@/lib/utils';
import { Button } from '@/components/ui/button';
import {
  entryRawText,
  formatEntryTime,
  levelPasses,
  parseConsoleLines,
  type ConsoleEntry,
  type ConsoleLevel,
  type LevelFilter,
  type RawConsoleLine,
} from '@/lib/consoleLog';

interface GameLogBatchEvent {
  lines: { line: string; stream: 'stdout' | 'stderr' }[];
  dropped_lines: number;
  instance_id: string;
}

interface Props {
  instanceId: string;
  className?: string;
  /** Pre-populated log buffer from the process controller, so historical
   *  logs appear immediately when the Console tab is opened. */
  logBuffer?: { line: string; stream: string; instance_id: string }[];
}

const MAX_LINES = 10000;
/** Rows rendered at once; older rows are behind "Show earlier". */
const PAGE_ROWS = 1000;

const LEVEL_BADGE: Record<ConsoleLevel, string> = {
  FATAL: 'bg-destructive text-destructive-foreground',
  ERROR: 'bg-destructive/20 text-destructive',
  WARN: 'bg-amber-500/20 text-amber-600 dark:text-amber-400',
  INFO: 'bg-muted text-muted-foreground',
  DEBUG: 'bg-muted text-muted-foreground opacity-70',
  TRACE: 'bg-muted text-muted-foreground opacity-60',
};

const FILTERS: { id: LevelFilter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'warnings', label: 'Warnings+' },
  { id: 'errors', label: 'Errors' },
];

function toRaw(l: { line: string; stream: string }): RawConsoleLine {
  return { line: l.line, stream: l.stream };
}

function ConsoleRow({
  entry,
  expanded,
  onToggle,
}: {
  entry: ConsoleEntry;
  expanded: boolean;
  onToggle: () => void;
}) {
  const severe = entry.level === 'ERROR' || entry.level === 'FATAL';
  const traceLines = entry.throwable ? entry.throwable.split('\n').length : 0;
  return (
    <div
      data-testid="console-entry"
      data-level={entry.level}
      className={cn(
        'border-l-2 py-0.5 pl-2',
        severe ? 'border-destructive bg-destructive/5' : entry.level === 'WARN' ? 'border-amber-500' : 'border-transparent',
      )}
    >
      <div className="flex items-start gap-2">
        <span className="shrink-0 text-muted-foreground">{formatEntryTime(entry)}</span>
        <span className={cn('w-12 shrink-0 rounded px-1 text-center text-[10px] font-semibold leading-4', LEVEL_BADGE[entry.level])}>
          {entry.level}
        </span>
        <div className="min-w-0 flex-1">
          <span className={cn('whitespace-pre-wrap break-words', severe && 'font-medium text-destructive')}>
            {entry.message}
          </span>
          {(entry.logger || entry.thread) && (
            <span className="ml-2 text-[10px] text-muted-foreground/70">
              {[entry.logger, entry.thread].filter(Boolean).join(' · ')}
            </span>
          )}
          {traceLines > 0 && (
            <button type="button" onClick={onToggle} className="ml-2 text-[10px] text-primary hover:underline" aria-expanded={expanded}>
              {expanded ? 'Hide stack trace' : `Show stack trace (${traceLines} lines)`}
            </button>
          )}
          {expanded && entry.throwable && (
            <pre className="mt-1 overflow-x-auto whitespace-pre-wrap break-words text-muted-foreground">{entry.throwable}</pre>
          )}
        </div>
      </div>
    </div>
  );
}

export function ConsoleView({ instanceId, className, logBuffer }: Props) {
  const [logs, setLogs] = useState<RawConsoleLine[]>(() => {
    if (logBuffer && logBuffer.length > 0) {
      return logBuffer.map(toRaw).slice(-MAX_LINES);
    }
    return [];
  });
  const [autoScroll, setAutoScroll] = useState(true);
  const [raw, setRaw] = useState(false);
  const [levelFilter, setLevelFilter] = useState<LevelFilter>('all');
  const [query, setQuery] = useState('');
  const [rows, setRows] = useState(PAGE_ROWS);
  // Entries whose stack-trace state differs from the default (open for errors).
  const [flipped, setFlipped] = useState<Set<number>>(new Set());
  const endRef = useRef<HTMLDivElement>(null);

  // Listen for bounded live-log batches, filtered by this instance.
  useEffect(() => {
    const unlisten = listen<GameLogBatchEvent>('game-log-batch', (e) => {
      if (e.payload.instance_id !== instanceId) return;
      setLogs((prev) => {
        const incoming = e.payload.lines.map(toRaw);
        if (e.payload.dropped_lines > 0) {
          incoming.unshift({
            line: `[Agora] Live console skipped ${e.payload.dropped_lines} log lines to keep the game responsive. Check the instance logs for persisted output.`,
            stream: 'stderr',
          });
        }
        const next = [...prev, ...incoming];
        return next.length > MAX_LINES ? next.slice(-MAX_LINES) : next;
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [instanceId]);

  const entries = useMemo(() => parseConsoleLines(logs), [logs]);

  const needle = query.trim().toLowerCase();
  const visible = useMemo(
    () =>
      entries.filter(
        (e) =>
          levelPasses(e.level, levelFilter) &&
          (!needle || entryRawText(e).toLowerCase().includes(needle)),
      ),
    [entries, levelFilter, needle],
  );
  const shown = visible.length > rows ? visible.slice(visible.length - rows) : visible;
  const hidden = visible.length - shown.length;

  // Raw view: exactly the captured lines of the visible entries.
  const rawText = useMemo(() => shown.map(entryRawText).join('\n'), [shown]);

  useEffect(() => {
    if (autoScroll) endRef.current?.scrollIntoView?.({ behavior: 'smooth' });
  }, [logs, autoScroll, raw]);

  const onScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
    setAutoScroll(atBottom);
  }, []);

  const clear = () => {
    setLogs([]);
    setFlipped(new Set());
    setRows(PAGE_ROWS);
  };

  const copy = (all: boolean) => {
    const text = (all ? entries : visible).map(entryRawText).join('\n');
    void navigator.clipboard?.writeText(text);
  };

  const isExpanded = (e: ConsoleEntry) => {
    const byDefault = e.level === 'ERROR' || e.level === 'FATAL';
    return flipped.has(e.id) ? !byDefault : byDefault;
  };
  const toggleTrace = (id: number) =>
    setFlipped((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  return (
    <div className={cn('rounded-lg border border-border bg-background', className)}>
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-1.5">
        <div className="flex gap-1" role="group" aria-label="Level filter">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              type="button"
              onClick={() => setLevelFilter(f.id)}
              aria-pressed={levelFilter === f.id}
              className={cn(
                'rounded px-1.5 py-0.5 text-xs font-medium',
                levelFilter === f.id ? 'bg-primary/20 text-primary' : 'text-muted-foreground',
              )}
            >
              {f.label}
            </button>
          ))}
        </div>
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search log"
          aria-label="Search log"
          className="h-6 w-36 rounded border border-border bg-transparent px-2 text-xs"
        />
        <div className="flex-1" />
        <span className="text-xs text-muted-foreground">{visible.length} entries</span>
        <Button variant="ghost" size="sm" onClick={() => setRaw((v) => !v)} className="h-6 text-xs" aria-pressed={raw}>
          {raw ? 'Readable' : 'Raw'}
        </Button>
        <Button variant="ghost" size="sm" onClick={() => copy(false)} className="h-6 text-xs">
          Copy
        </Button>
        <Button variant="ghost" size="sm" onClick={() => copy(true)} className="h-6 text-xs">
          Copy all
        </Button>
        <Button variant="ghost" size="sm" onClick={clear} className="h-6 text-xs">
          Clear
        </Button>
      </div>
      <div onScroll={onScroll} className="max-h-96 overflow-auto p-2 font-mono text-xs leading-relaxed">
        {hidden > 0 && (
          <button
            type="button"
            onClick={() => setRows((n) => n + PAGE_ROWS)}
            className="mb-1 text-xs text-primary hover:underline"
          >
            Show {Math.min(hidden, PAGE_ROWS)} earlier ({hidden} hidden)
          </button>
        )}
        {raw ? (
          <pre data-testid="console-raw" className="whitespace-pre-wrap break-words">
            {rawText}
          </pre>
        ) : (
          shown.map((e) => (
            <ConsoleRow key={e.id} entry={e} expanded={isExpanded(e)} onToggle={() => toggleTrace(e.id)} />
          ))
        )}
        <div ref={endRef} />
      </div>
    </div>
  );
}
