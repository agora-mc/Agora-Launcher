import { describe, expect, it } from 'vitest';
import { instanceReadiness, type ReadinessInput } from './instanceReadiness';

const base: ReadinessInput = {
  minecraftVersion: '1.21.1',
  loader: 'fabric',
  loaderVersion: '0.16.9',
  javaPath: null,
  javaMode: 'automatic',
  javaMajor: 21,
  health: { problems: 0, warnings: 0 },
  packGaps: 0,
  busy: false,
};

describe('instanceReadiness', () => {
  it('says ready when the game, mods and Java are all in order', () => {
    const r = instanceReadiness(base);
    expect(r.level).toBe('ready');
    expect(r.headline).toBe('Ready to play');
    expect(r.details).toContain('Minecraft 1.21.1 with Fabric 0.16.9');
    expect(r.details).toContain('Mods look healthy');
    expect(r.details).toContain("Java 21 downloads on first launch if it isn't already installed");
  });

  it('blocks on mod problems and counts them plainly', () => {
    const r = instanceReadiness({ ...base, health: { problems: 2, warnings: 1 } });
    expect(r.level).toBe('blocked');
    expect(r.headline).toBe('Not ready: 2 mod problems to fix');
    expect(r.details).toContain('2 mod problems to fix');
  });

  it('uses the singular for one problem', () => {
    const r = instanceReadiness({ ...base, health: { problems: 1, warnings: 0 } });
    expect(r.headline).toBe('Not ready: 1 mod problem to fix');
  });

  it('is almost ready with warnings to review, not ready', () => {
    const r = instanceReadiness({ ...base, health: { problems: 0, warnings: 1 } });
    expect(r.level).toBe('check');
    expect(r.headline).toBe('Almost ready: 1 warning to review');
  });

  it('does not claim mods are healthy before any health check ran', () => {
    const r = instanceReadiness({ ...base, health: null });
    expect(r.level).toBe('check');
    expect(r.details).toContain('Mods not checked yet');
    expect(r.details).not.toContain('Mods look healthy');
  });

  it('reports blocked, not almost ready, when busy even with warnings present', () => {
    const r = instanceReadiness({ ...base, busy: true, health: { problems: 0, warnings: 3 } });
    expect(r.level).toBe('blocked');
    expect(r.headline).toBe('Not ready: something is still running on this instance');
  });

  it('blocks while something is still running on the instance', () => {
    const r = instanceReadiness({ ...base, busy: true });
    expect(r.level).toBe('blocked');
    expect(r.headline).toBe('Not ready: something is still running on this instance');
  });

  it('blocks when the Minecraft version is blank or only spaces', () => {
    for (const minecraftVersion of ['', '   ']) {
      const r = instanceReadiness({ ...base, minecraftVersion });
      expect(r.level).toBe('blocked');
      expect(r.headline).toBe('Not ready: the Minecraft version is missing');
    }
  });

  it('blocks when the loader is not recorded at all', () => {
    const r = instanceReadiness({ ...base, loader: '  ' });
    expect(r.level).toBe('blocked');
    expect(r.headline).toBe('Not ready: the mod loader is not recorded');
  });

  it('blocks a modded instance with no loader version, naming the loader', () => {
    const r = instanceReadiness({ ...base, loaderVersion: '' });
    expect(r.level).toBe('blocked');
    expect(r.headline).toBe('Not ready: the Fabric loader version is missing');
  });

  it('reads loader names regardless of case and says vanilla for a plain game', () => {
    const r = instanceReadiness({ ...base, loader: 'NeoForge', loaderVersion: '21.1.3' });
    expect(r.details).toContain('Minecraft 1.21.1 with NeoForge 21.1.3');

    const vanilla = instanceReadiness({ ...base, loader: 'vanilla', loaderVersion: '' });
    expect(vanilla.level).toBe('ready');
    expect(vanilla.details).toContain('Minecraft 1.21.1, no mod loader');
  });

  it('keeps an unknown loader readable rather than inventing a name', () => {
    const r = instanceReadiness({ ...base, loader: 'Mystery', loaderVersion: '1' });
    expect(r.details).toContain('Minecraft 1.21.1 with mystery 1');
  });

  it('says the instance\'s own Java path wins over the global mode', () => {
    const r = instanceReadiness({ ...base, javaPath: 'C:\\jdk\\bin\\java.exe', javaMode: 'manual' });
    expect(r.details).toContain('Java: your own install (set in Java & Args)');
    expect(r.details.join(' ')).not.toContain('automatic downloads are off');
  });

  it('describes each global Java mode without promising a download it will not do', () => {
    expect(instanceReadiness({ ...base, javaMode: 'prompt' }).details)
      .toContain('Java 21 is needed: Agora will ask before downloading it');
    expect(instanceReadiness({ ...base, javaMode: 'manual' }).details)
      .toContain('Java 21 is needed: automatic downloads are off, so install it or pick it in Java & Args');
  });

  it('makes no download promise while the Java mode is unknown', () => {
    const r = instanceReadiness({ ...base, javaMode: null });
    expect(r.details).toContain('Java 21 is needed');
    expect(r.details.join(' ')).not.toMatch(/downloads|download/);
  });

  it('is not ready while a pack mod is missing, even when the installed mods look healthy', () => {
    const r = instanceReadiness({ ...base, packGaps: 1 });
    expect(r.level).toBe('blocked');
    expect(r.headline).toBe('Not ready: 1 pack mod is not installed');
    expect(r.details).toContain('1 pack mod not installed');
    expect(r.details).toContain('Mods look healthy');
    expect(r.details).not.toContain('Ready to play');
  });

  it('counts missing pack mods in the plural', () => {
    const r = instanceReadiness({ ...base, packGaps: 2 });
    expect(r.headline).toBe('Not ready: 2 pack mods are not installed');
    expect(r.details).toContain('2 pack mods not installed');
  });

  it('does not claim ready before the instance record says whether pack mods are missing', () => {
    const r = instanceReadiness({ ...base, packGaps: null });
    expect(r.level).toBe('check');
    expect(r.headline).toBe('Almost ready: pack mods not checked yet');
    expect(r.details).toContain('Pack mods not checked yet');
    expect(r.level).not.toBe('ready');
  });

  it('still reports busy first when a pack mod is also missing', () => {
    const r = instanceReadiness({ ...base, busy: true, packGaps: 3 });
    expect(r.headline).toBe('Not ready: something is still running on this instance');
  });

  it('treats an empty Java path as no override', () => {
    // The caller passes null for "no override"; an empty string from storage must not look like a path.
    const r = instanceReadiness({ ...base, javaPath: '' });
    expect(r.details.join(' ')).not.toContain('your own install');
  });
});
