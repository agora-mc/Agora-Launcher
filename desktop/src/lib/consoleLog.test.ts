import { describe, expect, it } from 'vitest';
import { levelPasses, parseConsoleLines, type RawConsoleLine } from './consoleLog';

const out = (...lines: string[]): RawConsoleLine[] => lines.map((line) => ({ line, stream: 'stdout' }));
const err = (...lines: string[]): RawConsoleLine[] => lines.map((line) => ({ line, stream: 'stderr' }));

describe('parseConsoleLines', () => {
  it('parses a log4j event split over three output lines', () => {
    const entries = parseConsoleLines(
      out(
        '<log4j:Event logger="net.minecraft.client.Minecraft" timestamp="1790743960312" level="INFO" thread="Render thread">',
        '  <log4j:Message><![CDATA[Backend library: LWJGL version 3.4.1-snapshot]]></log4j:Message>',
        '</log4j:Event>',
      ),
    );
    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({
      level: 'INFO',
      thread: 'Render thread',
      logger: 'net.minecraft.client.Minecraft',
      time: 1790743960312,
      message: 'Backend library: LWJGL version 3.4.1-snapshot',
      structured: true,
    });
    expect(entries[0].rawLines).toHaveLength(3);
  });

  it('keeps a trailing bracket and handles CDATA escapes and entities', () => {
    const entries = parseConsoleLines(
      out(
        '<log4j:Event logger="a&amp;b" timestamp="1" level="WARN" thread="main"><log4j:Message><![CDATA[workarounds: [A, B]]]></log4j:Message></log4j:Event>',
        '<log4j:Event logger="x" timestamp="2" level="INFO" thread="main"><log4j:Message><![CDATA[end]]]]><![CDATA[> done]]></log4j:Message></log4j:Event>',
        '<log4j:Event logger="x" timestamp="3" level="INFO" thread="main"><log4j:Message>1 &lt; 2 &amp;&amp; &#65;</log4j:Message></log4j:Event>',
      ),
    );
    expect(entries.map((e) => e.message)).toEqual(['workarounds: [A, B]', 'end]]> done', '1 < 2 && A']);
    expect(entries[0].logger).toBe('a&b');
    expect(entries[0].level).toBe('WARN');
  });

  it('keeps a multi-line message and the throwable', () => {
    const entries = parseConsoleLines(
      out(
        '<log4j:Event logger="mod" timestamp="5" level="ERROR" thread="main">',
        '<log4j:Message><![CDATA[Failed to load',
        'second line]]></log4j:Message>',
        '<log4j:Throwable><![CDATA[java.lang.IllegalStateException: boom',
        '\tat mod.Main.run(Main.java:1)',
        ']]></log4j:Throwable>',
        '</log4j:Event>',
      ),
    );
    expect(entries).toHaveLength(1);
    expect(entries[0].message).toBe('Failed to load\nsecond line');
    expect(entries[0].throwable).toBe('java.lang.IllegalStateException: boom\n\tat mod.Main.run(Main.java:1)');
  });

  it('shows the duplicate-ASM stderr failure as one error entry with its trace', () => {
    const entries = parseConsoleLines(
      err(
        'Exception in thread "main" java.lang.ExceptionInInitializerError',
        '\tat net.fabricmc.loader.impl.launch.knot.KnotClient.main(KnotClient.java:23)',
        'Caused by: java.lang.IllegalStateException: duplicate ASM classes found on classpath: a.jar, b.jar',
        '\tat net.fabricmc.loader.impl.util.LoaderUtil.verifyClasspath(LoaderUtil.java:83)',
        '\t... 1 more',
      ),
    );
    expect(entries).toHaveLength(1);
    expect(entries[0].level).toBe('ERROR');
    expect(entries[0].message).toContain('ExceptionInInitializerError');
    expect(entries[0].throwable).toContain('duplicate ASM classes found on classpath');
    expect(entries[0].rawLines).toHaveLength(5);
  });

  it('treats plain stderr as an error and plain stdout as info', () => {
    const entries = parseConsoleLines([
      { line: 'Something broke', stream: 'stderr' },
      { line: 'WARNING: A terminally deprecated method', stream: 'stderr' },
      { line: 'Setting user: Dev', stream: 'stdout' },
    ]);
    expect(entries.map((e) => e.level)).toEqual(['ERROR', 'WARN', 'INFO']);
  });

  it('classifies JVM notices on stderr as warnings, not errors', () => {
    const entries = parseConsoleLines(
      err(
        'OpenJDK 64-Bit Server VM warning: Sharing is only supported for boot loader classes',
        'Java HotSpot(TM) 64-Bit Server VM warning: Options -Xverify:none and -noverify were deprecated',
        'Ignoring option ZGenerational; support was removed in 24.0',
        'Exception in thread "main" java.lang.IllegalStateException: boom',
        'Something broke',
      ),
    );
    expect(entries.map((e) => e.level)).toEqual(['WARN', 'WARN', 'WARN', 'ERROR', 'ERROR']);
  });

  it('parses legacy bracketed lines', () => {
    const entries = parseConsoleLines(
      out(
        '[12:00:01] [main/INFO]: Loading Minecraft 1.12.2',
        '[12:00:02] [Server thread/WARN] [net.minecraftforge.Foo/]: Careful: here',
        '[12:00:03] [main/FATAL]: Bad',
      ),
    );
    expect(entries[0]).toMatchObject({ clock: '12:00:01', thread: 'main', level: 'INFO', message: 'Loading Minecraft 1.12.2' });
    expect(entries[1]).toMatchObject({ level: 'WARN', logger: 'net.minecraftforge.Foo/', message: 'Careful: here' });
    expect(entries[2].level).toBe('FATAL');
  });

  it('never loses text from malformed or truncated XML', () => {
    const input = out(
      '<log4j:Event logger="a" timestamp="1" level="INFO" thread="t">',
      '  <log4j:Message><![CDATA[never closed',
      '<log4j:Event logger="b" timestamp="2" level="INFO" thread="t"><log4j:Message><![CDATA[ok]]></log4j:Message></log4j:Event>',
      '<log4j:Event logger="c" timestamp="3" level="INFO" thread="t">',
      '<log4j:Message><![CDATA[in flight',
    );
    const entries = parseConsoleLines(input);
    expect(entries.flatMap((e) => e.rawLines)).toEqual(input.map((l) => l.line));
    expect(entries.find((e) => e.message === 'ok')?.structured).toBe(true);
  });

  it('keeps every raw line for a mixed stream', () => {
    const input = [
      ...out('plain', '<log4j:Event logger="a" timestamp="1" level="INFO" thread="t">', '<log4j:Message><![CDATA[x]]></log4j:Message>', '</log4j:Event>'),
      ...err('oops', '\tat x.Y(Y.java:1)'),
    ];
    const entries = parseConsoleLines(input);
    expect(entries.flatMap((e) => e.rawLines)).toEqual(input.map((l) => l.line));
  });
});

describe('levelPasses', () => {
  it('filters by severity', () => {
    expect(levelPasses('INFO', 'all')).toBe(true);
    expect(levelPasses('INFO', 'warnings')).toBe(false);
    expect(levelPasses('WARN', 'warnings')).toBe(true);
    expect(levelPasses('WARN', 'errors')).toBe(false);
    expect(levelPasses('FATAL', 'errors')).toBe(true);
  });
});
