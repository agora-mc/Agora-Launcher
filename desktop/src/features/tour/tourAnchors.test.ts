import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { ANCHOR_PATTERN } from './tourDom';
import { TOUR_STEPS, watchedAnchors } from './tourModel';

/**
 * The tour's anchors are strings shared between the script and eight unrelated
 * components, so nothing in the type system notices when one side is renamed.
 * This audit does: every anchor the script names must be declared by some
 * element, and every declared anchor must still be used.
 *
 * Declarations are read two ways. A literal `data-tour="x"` is the normal one.
 * Where an anchor is applied conditionally or from a lookup table (the sidebar
 * nav, the first row of the version list) the attribute value is not a literal,
 * so a quoted anchor string anywhere in a file that also mentions `data-tour`
 * counts as a declaration too — for the "is it declared" direction only, so a
 * stale attribute cannot hide behind an unrelated string.
 */

const TOUR_ROOT = dirname(fileURLToPath(import.meta.url));
const SRC_ROOT = resolve(TOUR_ROOT, '..', '..');

function sourceFiles(directory: string): string[] {
  const found: string[] = [];
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    if (statSync(path).isDirectory()) {
      // The tour's own sources talk *about* `data-tour` (selectors, comments)
      // without declaring anchors, so scanning them would invent anchors.
      if (entry === 'node_modules' || path === TOUR_ROOT) continue;
      found.push(...sourceFiles(path));
    } else if (/\.tsx?$/.test(entry) && !/\.(test|spec)\.tsx?$/.test(entry)) {
      found.push(path);
    }
  }
  return found;
}

function collectDeclarations() {
  const attributeLiterals = new Set<string>();
  const anyQuotedString = new Set<string>();

  for (const file of sourceFiles(SRC_ROOT)) {
    const contents = readFileSync(file, 'utf8');
    if (!contents.includes('data-tour')) continue;
    for (const match of contents.matchAll(/data-tour="([^"]+)"/g)) {
      if (ANCHOR_PATTERN.test(match[1])) attributeLiterals.add(match[1]);
    }
    for (const match of contents.matchAll(/'([a-z0-9]+(?:-[a-z0-9]+)*)'/g)) {
      anyQuotedString.add(match[1]);
    }
  }

  return { attributeLiterals, anyQuotedString };
}

const referenced = new Set<string>();
for (const step of TOUR_STEPS) {
  step.anchors?.forEach((anchor) => referenced.add(anchor));
  if (step.gate) referenced.add(step.gate);
  watchedAnchors(step.advance).forEach((anchor) => referenced.add(anchor));
}

describe('tour anchors', () => {
  const { attributeLiterals, anyQuotedString } = collectDeclarations();

  it('finds the anchors declared in the app', () => {
    // A smoke check on the scan itself: if this ever comes back empty the two
    // assertions below would pass for the wrong reason.
    expect(attributeLiterals.size).toBeGreaterThan(10);
  });

  it('has every anchor the script names declared by an element', () => {
    const missing = [...referenced].filter(
      (anchor) => !attributeLiterals.has(anchor) && !anyQuotedString.has(anchor),
    );
    expect(missing, `no element carries data-tour for: ${missing.join(', ')}`).toEqual([]);
  });

  it('has no data-tour attribute the script no longer uses', () => {
    const unused = [...attributeLiterals].filter((anchor) => !referenced.has(anchor));
    expect(unused, `declared but unused: ${unused.join(', ')}`).toEqual([]);
  });

  it('uses kebab-case anchor names, so plain attribute selectors are safe', () => {
    for (const anchor of referenced) {
      expect(ANCHOR_PATTERN.test(anchor), `"${anchor}" is not a kebab-case anchor`).toBe(true);
    }
  });
});
