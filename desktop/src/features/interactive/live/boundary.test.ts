import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, normalize } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Live boundary contract (SOL-2 §14.3): only `live/` may map backend DTOs or
 * import the app layer. `domain/`, `visual/`, and `lab/` must never import
 * from `live/`. The automated boundary check enforces this in CI; this test
 * statically asserts the same for fast feedback.
 */

const INTERACTIVE = join(process.cwd(), 'src', 'features', 'interactive');

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

describe('live boundary', () => {
  it('domain/, visual/, and lab/ never import from live/', () => {
    const violations: string[] = [];
    for (const area of ['domain', 'visual', 'lab']) {
      const root = join(INTERACTIVE, area);
      for (const file of collectTs(root)) {
        const source = readFileSync(file, 'utf8');
        for (const spec of importsOf(source)) {
          if (spec.includes('/live/') || spec.includes('/live')) {
            violations.push(`${file.replace(/\\/g, '/').split('/src/')[1]}: imports '${spec}'`);
          }
        }
      }
    }
    expect(violations).toEqual([]);
  });

  it('live/ imports only tauri, domain, visual, and its own modules (plus react)', () => {
    // `react-dom` sits in the same tier as `react`: a rendering primitive, not an
    // app-boundary or operation import. `createPortal` is needed because a fixed
    // overlay nested inside a backdrop-filtered scroll container is positioned
    // against that ancestor rather than the viewport.
    const root = join(INTERACTIVE, 'live');
    const violations: string[] = [];
    const allowedRoots = [join(INTERACTIVE, 'live'), join(INTERACTIVE, 'domain'), join(INTERACTIVE, 'visual')];
    for (const file of collectTs(root)) {
      if (/\.(test|spec)\.(ts|tsx)$/.test(file)) continue; // test files get test-framework allowances
      const source = readFileSync(file, 'utf8');
      for (const spec of importsOf(source)) {
        const ok =
          spec === 'react'
          || spec === 'react/'
          || spec === 'react-dom'
          || spec.startsWith('@/lib/tauri')
          || spec.includes('/domain/')
          || spec.includes('/visual/')
          || spec.startsWith('./')
          || (spec.startsWith('../')
            && (() => {
              // Resolve a sibling specifier and allow it only when it stays
              // inside live/, domain/, or visual/ (e.g. operationBridges ->
              // ../intentController is a live-internal import).
              const resolved = normalize(join(dirname(file), spec));
              return allowedRoots.some((rootPath) => resolved.startsWith(rootPath));
            })());
        if (!ok) {
          violations.push(`${file.replace(/\\/g, '/').split('/src/')[1]}: imports '${spec}'`);
        }
      }
    }
    expect(violations).toEqual([]);
  });
});
