import { useState, useEffect, useRef, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cn } from '@/lib/utils';
import { Button } from '@/components/ui/button';

interface GameLogEvent {
  line: string;
  stream: 'stdout' | 'stderr';
  instance_id: string;
}

interface LogLine {
  line: string;
  stream: string;
  level: string;
}

interface Props {
  instanceId: string;
  className?: string;
  /** Pre-populated log buffer from the process controller, so historical
   *  logs appear immediately when the Console tab is opened. */
  logBuffer?: { line: string; stream: string; instance_id: string }[];
}

const MAX_LINES = 10000;

function detectLevel(line: string): string {
  if (line.includes('[ERROR]')) return 'ERROR';
  if (line.includes('[WARN]')) return 'WARN';
  if (line.includes('[DEBUG]')) return 'DEBUG';
  return 'INFO';
}

function toLogLine(l: { line: string; stream: string }): LogLine {
  return { line: l.line, stream: l.stream, level: detectLevel(l.line) };
}

export function ConsoleView({ instanceId, className, logBuffer }: Props) {
  const [logs, setLogs] = useState<LogLine[]>(() => {
    if (logBuffer && logBuffer.length > 0) {
      return logBuffer.map(toLogLine).slice(-MAX_LINES);
    }
    return [];
  });
  const [autoScroll, setAutoScroll] = useState(true);
  const [filter, setFilter] = useState<Set<string>>(new Set(['INFO', 'WARN', 'ERROR', 'DEBUG']));
  const endRef = useRef<HTMLDivElement>(null);

  // Listen for live game-log events, filtered by this instance.
  useEffect(() => {
    const unlisten = listen<GameLogEvent>('game-log', (e) => {
      if (e.payload.instance_id !== instanceId) return;
      setLogs((prev) => {
        const next = [...prev, toLogLine(e.payload)];
        return next.length > MAX_LINES ? next.slice(-MAX_LINES) : next;
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [instanceId]);

  useEffect(() => {
    if (autoScroll) endRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [logs, autoScroll]);

  const onScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
    setAutoScroll(atBottom);
  }, []);

  const clear = () => setLogs([]);

  const copyVisible = useCallback(() => {
    const visible = filtered().map((l) => l.line).join('\n');
    navigator.clipboard.writeText(visible);
  }, [logs, filter]);

  const filtered = () => logs.filter((l) => filter.has(l.level));

  const toggle = (level: string) => {
    setFilter((prev) => {
      const next = new Set(prev);
      next.has(level) ? next.delete(level) : next.add(level);
      return next;
    });
  };

  const levelColor: Record<string, string> = {
    ERROR: 'text-destructive',
    WARN: 'text-amber-500',
    DEBUG: 'text-muted-foreground',
    INFO: 'text-foreground',
  };

  const visible = filtered();

  return (
    <div className={cn('rounded-lg border border-border bg-background', className)}>
      <div className="flex items-center gap-2 border-b border-border px-3 py-1.5">
        {['INFO', 'WARN', 'ERROR', 'DEBUG'].map((l) => (
          <button
            key={l}
            onClick={() => toggle(l)}
            className={cn(
              'rounded px-1.5 py-0.5 text-xs font-medium',
              filter.has(l) ? 'bg-primary/20 text-primary' : 'text-muted-foreground opacity-50',
            )}
          >
            {l}
          </button>
        ))}
        <div className="flex-1" />
        <span className="text-xs text-muted-foreground">{visible.length} lines</span>
        <Button variant="ghost" size="sm" onClick={copyVisible} className="h-6 text-xs">
          Copy
        </Button>
        <Button variant="ghost" size="sm" onClick={clear} className="h-6 text-xs">
          Clear
        </Button>
      </div>
      <div onScroll={onScroll} className="overflow-auto max-h-96 p-2 font-mono text-xs leading-relaxed">
        {visible.map((l, i) => (
          <div
            key={i}
            className={cn(
              levelColor[l.level],
              l.stream === 'stderr' ? 'border-l-2 border-destructive pl-1' : '',
            )}
          >
            {l.line}
          </div>
        ))}
        <div ref={endRef} />
      </div>
    </div>
  );
}
