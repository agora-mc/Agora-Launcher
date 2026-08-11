import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * SOL-1 BLOCKER 4 contract: shared visual components may only communicate
 * through declared `VisualIntent`s. Operation-like callback props (review,
 * apply, install, remove, restore, repair, launch, save) are forbidden — a
 * future live host must route the same intent through the live controller.
 * The import-boundary script also enforces this structurally; this test scans
 * the prop interfaces directly for regression coverage.
 */

const VISUAL_ROOT = join(process.cwd(), 'src', 'features', 'interactive', 'visual');

const OPERATION_LIKE = /\bon(Review|Apply|Install|Remove|Restore|Repair|Launch|Save|Toggle|Execute|Commit|Submit)\b\s*[:?]\s/;

function collectTsx(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      collectTsx(full, out);
    } else if (/\.tsx?$/.test(entry)) {
      out.push(full);
    }
  }
  return out;
}

describe('shared visual contract (SOL-1 BLOCKER 4)', () => {
  it('exposes no operation-like callback props outside the VisualIntent union', () => {
    const violations: string[] = [];
    for (const file of collectTsx(VISUAL_ROOT)) {
      if (file.endsWith('.test.tsx')) continue;
      const source = readFileSync(file, 'utf8');
      const match = OPERATION_LIKE.exec(source);
      if (match) {
        violations.push(`${file.replace(/\\/g, '/').split('src/')[1]}: forbidden callback prop '${match[1]}'`);
      }
    }
    expect(violations).toEqual([]);
  });

  it('emits a declared review-staged-changes intent for the review control', () => {
    // ChangeStaging is covered by its component test; assert the intent exists
    // in the closed union so the review route stays inside VisualIntent.
    const intentsPath = join(process.cwd(), 'src', 'features', 'interactive', 'domain', 'intents.ts');
    const intents = readFileSync(intentsPath, 'utf8');
    expect(intents).toMatch(/review-staged-changes/);
  });
});
