/**
 * Display-only parser for the game console stream.
 *
 * The launcher hands Minecraft a log4j config that makes the game print one
 * `<log4j:Event>` XML element per log call, often spread over several output
 * lines. This turns the raw captured lines into readable entries. It never
 * mutates or replaces the captured text: every entry keeps its original lines
 * in `rawLines`, and the persisted launch log / Crash Doctor evidence are
 * written by the backend from the untouched stream.
 */

export type ConsoleLevel = 'TRACE' | 'DEBUG' | 'INFO' | 'WARN' | 'ERROR' | 'FATAL';

export interface RawConsoleLine {
  line: string;
  stream: string;
}

export interface ConsoleEntry {
  /** Index of the first raw line; stable while the buffer only grows. */
  id: number;
  level: ConsoleLevel;
  stream: string;
  /** Epoch milliseconds, when the source carried a full timestamp. */
  time?: number;
  /** Wall-clock text (HH:MM:SS) from sources with no date, e.g. legacy Forge. */
  clock?: string;
  thread?: string;
  logger?: string;
  message: string;
  /** Stack trace / continuation text, without the message line. */
  throwable?: string;
  /** Exactly what was captured for this entry. */
  rawLines: string[];
  /** True when the entry came from log4j XML or a bracketed log format. */
  structured: boolean;
}

/** Give up on an unterminated `<log4j:Event>` after this many lines. */
const MAX_EVENT_LINES = 400;

const EVENT_OPEN = /^\s*<log4j:Event\b/;
const EVENT_CLOSE = '</log4j:Event>';
const LEGACY = /^\[(\d{1,2}:\d{2}:\d{2})\] \[([^\]/]+)\/([A-Za-z]+)\](?: \[([^\]]*)\])?: ?(.*)$/;
const STACK_CONTINUATION =
  /^(\s+at\s|\s*Caused by:|\s*Suppressed:|\s*\.\.\. \d+ (more|common frames omitted)|\s+\.\.\. )/;
/**
 * Notices the JVM itself prints on stderr that are not failures: "OpenJDK
 * 64-Bit Server VM warning: ...", "Ignoring option X; support was removed in N"
 * and plain "WARNING:" lines. Treating every stderr line as an error buries the
 * real ones in the Errors filter.
 */
const JVM_WARNING =
  /^\s*(WARNING\b|\[WARN)|\bVM warning:|^\s*(OpenJDK|Java HotSpot\(TM\)|Eclipse OpenJ9)\b.*\bwarning\b|^\s*Ignoring option\b/i;
const EXCEPTION_START = /^(Exception in thread\b|(?:[\w$]+\.)+[\w$]*(?:Exception|Error|Throwable)\b(?::|$))/;

export function normalizeLevel(value: string | undefined): ConsoleLevel | undefined {
  switch ((value ?? '').trim().toUpperCase()) {
    case 'TRACE':
    case 'FINEST':
    case 'FINER':
      return 'TRACE';
    case 'DEBUG':
    case 'FINE':
    case 'CONFIG':
      return 'DEBUG';
    case 'INFO':
      return 'INFO';
    case 'WARN':
    case 'WARNING':
      return 'WARN';
    case 'ERROR':
    case 'SEVERE':
      return 'ERROR';
    case 'FATAL':
      return 'FATAL';
    default:
      return undefined;
  }
}

const XML_ENTITIES: Record<string, string> = {
  lt: '<',
  gt: '>',
  amp: '&',
  quot: '"',
  apos: "'",
};

function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-fA-F]+|#\d+|[a-zA-Z]+);/g, (whole, body: string) => {
    if (body[0] === '#') {
      const code = body[1] === 'x' || body[1] === 'X' ? parseInt(body.slice(2), 16) : parseInt(body.slice(1), 10);
      try {
        return Number.isFinite(code) ? String.fromCodePoint(code) : whole;
      } catch {
        return whole;
      }
    }
    return XML_ENTITIES[body] ?? whole;
  });
}

/** Text content of an element body: CDATA sections verbatim, the rest entity-decoded. */
function decodeContent(body: string): string {
  let out = '';
  const re = /<!\[CDATA\[([\s\S]*?)\]\]>|([^<]+|<)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body)) !== null) {
    out += m[1] !== undefined ? m[1] : decodeEntities(m[2]);
  }
  return out;
}

/** Content between `<tag>` and `</tag>`; tolerates a missing close tag. */
function elementBody(xml: string, tag: string): string | undefined {
  const open = new RegExp(`<${tag}(?:\\s[^>]*)?>`).exec(xml);
  if (!open) return undefined;
  const start = open.index + open[0].length;
  // The close tag must be outside any CDATA section, so scan for it after
  // skipping CDATA blocks.
  const close = `</${tag}>`;
  let i = start;
  while (i < xml.length) {
    if (xml.startsWith('<![CDATA[', i)) {
      const end = xml.indexOf(']]>', i + 9);
      if (end < 0) return xml.slice(start);
      i = end + 3;
      continue;
    }
    if (xml.startsWith(close, i)) return xml.slice(start, i);
    i += 1;
  }
  return xml.slice(start);
}

function parseAttributes(tag: string): Record<string, string> {
  const attrs: Record<string, string> = {};
  const re = /([\w:.-]+)\s*=\s*"([^"]*)"/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(tag)) !== null) attrs[m[1]] = decodeEntities(m[2]);
  return attrs;
}

function plainEntry(id: number, raw: RawConsoleLine): ConsoleEntry {
  const text = raw.line;
  const legacy = LEGACY.exec(text);
  if (legacy) {
    return {
      id,
      level: normalizeLevel(legacy[3]) ?? 'INFO',
      stream: raw.stream,
      clock: legacy[1],
      thread: legacy[2],
      logger: legacy[4] || undefined,
      message: legacy[5],
      rawLines: [text],
      structured: true,
    };
  }
  let level: ConsoleLevel = 'INFO';
  if (text.startsWith('[Agora]')) level = 'WARN';
  else if (EXCEPTION_START.test(text)) level = 'ERROR';
  else if (raw.stream === 'stderr') level = JVM_WARNING.test(text) ? 'WARN' : 'ERROR';
  return { id, level, stream: raw.stream, message: text, rawLines: [text], structured: false };
}

/** Try to read a complete log4j event; `undefined` means it was malformed. */
function eventEntry(id: number, lines: string[], stream: string): ConsoleEntry | undefined {
  const xml = lines.join('\n');
  const open = /<log4j:Event\b([^>]*)>/.exec(xml);
  if (!open) return undefined;
  const messageBody = elementBody(xml, 'log4j:Message');
  if (messageBody === undefined) return undefined;
  const attrs = parseAttributes(open[1]);
  const throwableBody = elementBody(xml, 'log4j:Throwable');
  const ts = attrs.timestamp ? Number(attrs.timestamp) : NaN;
  const throwable = throwableBody === undefined ? undefined : decodeContent(throwableBody).replace(/^\n+|\s+$/g, '');
  return {
    id,
    level: normalizeLevel(attrs.level) ?? 'INFO',
    stream,
    time: Number.isFinite(ts) ? ts : undefined,
    thread: attrs.thread || undefined,
    logger: attrs.logger || undefined,
    message: decodeContent(messageBody),
    throwable: throwable || undefined,
    rawLines: lines,
    structured: true,
  };
}

/** Parse a captured console buffer into entries. Never drops or rewrites text. */
export function parseConsoleLines(lines: readonly RawConsoleLine[]): ConsoleEntry[] {
  const entries: ConsoleEntry[] = [];
  const xmlEntries = new WeakSet<ConsoleEntry>();
  let open: { id: number; lines: RawConsoleLine[] } | null = null;

  const emitPlain = (id: number, raws: RawConsoleLine[]) => {
    raws.forEach((raw, k) => {
      const prev = entries[entries.length - 1];
      if (prev && !xmlEntries.has(prev) && STACK_CONTINUATION.test(raw.line)) {
        // Stack frames belong to the line that started the trace.
        prev.throwable = prev.throwable === undefined ? raw.line : `${prev.throwable}\n${raw.line}`;
        prev.rawLines.push(raw.line);
        return;
      }
      entries.push(plainEntry(id + k, raw));
    });
  };

  /** Finish the open event: structured when well-formed, raw text otherwise. */
  const finish = () => {
    if (!open) return;
    const { id, lines: raws } = open;
    open = null;
    const parsed = eventEntry(
      id,
      raws.map((r) => r.line),
      raws[0].stream,
    );
    if (parsed) {
      xmlEntries.add(parsed);
      entries.push(parsed);
    } else {
      emitPlain(id, raws);
    }
  };

  for (let i = 0; i < lines.length; i += 1) {
    const raw = lines[i];
    if (open) {
      if (EVENT_OPEN.test(raw.line)) {
        // A new event began before the last one closed: keep the old text as-is.
        const { id, lines: raws } = open;
        open = null;
        emitPlain(id, raws);
      } else {
        open.lines.push(raw);
        if (raw.line.includes(EVENT_CLOSE)) finish();
        else if (open.lines.length >= MAX_EVENT_LINES) {
          const { id, lines: raws } = open;
          open = null;
          emitPlain(id, raws);
        }
        continue;
      }
    }
    if (EVENT_OPEN.test(raw.line)) {
      open = { id: i, lines: [raw] };
      if (raw.line.includes(EVENT_CLOSE)) finish();
    } else {
      emitPlain(i, [raw]);
    }
  }
  // An event still open at the end is in flight or truncated; show the text we
  // have rather than hiding it.
  if (open) {
    const { id, lines: raws } = open as { id: number; lines: RawConsoleLine[] };
    emitPlain(id, raws);
  }
  return entries;
}

export function entryRawText(entry: ConsoleEntry): string {
  return entry.rawLines.join('\n');
}

export type LevelFilter = 'all' | 'warnings' | 'errors';

export function levelPasses(level: ConsoleLevel, filter: LevelFilter): boolean {
  if (filter === 'all') return true;
  if (filter === 'errors') return level === 'ERROR' || level === 'FATAL';
  return level === 'WARN' || level === 'ERROR' || level === 'FATAL';
}

export function formatEntryTime(entry: ConsoleEntry): string {
  if (entry.time !== undefined) {
    const d = new Date(entry.time);
    const p = (n: number) => String(n).padStart(2, '0');
    return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  }
  return entry.clock ?? '';
}
