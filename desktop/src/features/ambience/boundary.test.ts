import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, normalize } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Ambience boundary contract: the ambience layer must never import the
 * interactive layer, any page, or the Tauri side (beyond its own settings
 * module). The automated `check-ambience-boundaries.mjs` enforces this in CI;
 * this test statically asserts the same for fast feedback.
 */

const AMBIENCE = join(process.cwd(), 'src', 'features', 'ambience');

function collectTs(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      collectTs(full, out);
    } else if (/\.(ts|tsx)$/.test(entry)) {
      out.push(full);
    }
  }
  return out;
}

function importsOf(source: string): string[] {
  const specs: string[] = [];
  const re = /(?:import|export)\s+(?:[^'"]*?\s+from\s+)?['"]([^'"]+)['"]/g;
  let match;
  while ((match = re.exec(source)) !== null) specs.push(match[1]);
  return specs;
}

describe('ambience boundary', () => {
  it('features/ambience/ imports only react, its own modules, and its own settings read', () => {
    const violations: string[] = [];
    for (const file of collectTs(AMBIENCE)) {
      if (/\.(test|spec)\.(ts|tsx)$/.test(file)) continue;
      const rel = file.replace(/\\/g, '/').split('/src/')[1];
      const source = readFileSync(file, 'utf8');
      for (const spec of importsOf(source)) {
        if (spec === 'react') continue;
        if (spec.startsWith('./') || spec.startsWith('../')) {
          const resolved = normalize(join(dirname(file), spec)).replace(/\\/g, '/');
          if (!resolved.startsWith(AMBIENCE.replace(/\\/g, '/') + '/')) {
            violations.push(`${rel}: relative import escapes ambience ('${spec}')`);
          }
          continue;
        }
        if (spec.startsWith('@/lib/tauri')) {
          // permitted settings/journal storage — ambienceSettings + journalStorage
          if (rel === 'features/ambience/ambienceSettings.ts' || rel === 'features/ambience/journalStorage.ts') {
            continue;
          }
          violations.push(`${rel}: imports @/lib/tauri outside ambienceSettings.ts/journalStorage.ts ('${spec}')`);
          continue;
        }
        if (spec.startsWith('@/features/ambience/')) continue;
        violations.push(`${rel}: imports outside the ambience allowlist ('${spec}')`);
      }
    }
    expect(violations).toEqual([]);
  });
});
