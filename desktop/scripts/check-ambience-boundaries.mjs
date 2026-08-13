#!/usr/bin/env node
/**
 * Import-boundary check for the ambience feature (V5-PORT-PLAN §3).
 *
 * `features/ambience/` is a NEW top-level feature that must never reach into
 * the interactive layer, any page, or the Tauri side:
 *
 *   - relative imports must resolve INSIDE `features/ambience/`;
 *   - `react` is allowed (the React components);
 *   - `@/lib/tauri` is allowed ONLY from `ambienceSettings.ts`, and only for
 *     reading its own settings (getSetting/setSetting) — the one permitted
 *     outside-in dependency;
 *   - everything else — `features/interactive/**`, `pages/**`, other `@/lib/*`
 *     commands, `@/components/**` — is a violation.
 *
 * Usage:
 *   node scripts/check-ambience-boundaries.mjs                  # normal mode
 *   node scripts/check-ambience-boundaries.mjs --root <dir> --fixtures
 *       # fixture mode: every scanned file must produce >=1 violation
 *
 * A boundary with no failing fixture is not enforced, it is just documented —
 * so negative fixtures live in `scripts/boundary-fixtures/ambience/` and the
 * fixture mode proves each one is caught.
 */

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, isAbsolute, join, normalize, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const DESKTOP_ROOT = resolve(SCRIPT_DIR, '..');

const args = process.argv.slice(2);
const rootIndex = args.indexOf('--root');
const ROOT = rootIndex >= 0
  ? resolve(process.cwd(), args[rootIndex + 1])
  : resolve(DESKTOP_ROOT, 'src', 'features', 'ambience');
const FIXTURES_MODE = args.includes('--fixtures');

function collectTs(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) collectTs(full, out);
    else if (/\.(ts|tsx)$/.test(entry)) out.push(full);
  }
  return out;
}

function importsOf(source) {
  const specs = [];
  const re = /(?:import|export)\s+(?:[^'"]*?\s+from\s+)?['"]([^'"]+)['"]/g;
  let match;
  while ((match = re.exec(source)) !== null) specs.push(match[1]);
  return specs;
}

function violationsFor(file, source) {
  const out = [];
  const rel = relative(ROOT, file);
  const isSettings = rel.replace(/\\/g, '/') === 'ambienceSettings.ts';
  for (const spec of importsOf(source)) {
    if (spec === 'react') continue;
    if (spec.startsWith('./') || spec.startsWith('../')) {
      const resolved = normalize(join(dirname(file), spec));
      const insideRoot = resolved.startsWith(ROOT + sep) || resolved === ROOT;
      if (!insideRoot) {
        out.push(`relative import escapes features/ambience/: '${spec}'`);
      }
      continue;
    }
    if (spec.startsWith('@/lib/tauri')) {
      if (isSettings) continue; // the one permitted settings read
      out.push(`imports @/lib/tauri outside ambienceSettings.ts: '${spec}'`);
      continue;
    }
    out.push(`imports outside the ambience allowlist: '${spec}'`);
  }
  return out;
}

const files = collectTs(ROOT).filter((f) => !/\.(test|spec)\.(ts|tsx)$/.test(f));
let totalViolations = 0;
let scannedWithViolations = 0;
let silentFiles = 0;

for (const file of files) {
  const source = readFileSync(file, 'utf8');
  const violations = violationsFor(file, source);
  const rel = relative(DESKTOP_ROOT, file).split(sep).join('/');
  if (FIXTURES_MODE) {
    if (violations.length === 0) {
      console.error(`FIXTURE NOT VIOLATING: ${rel}`);
      silentFiles++;
    } else {
      scannedWithViolations++;
    }
  } else {
    if (violations.length > 0) {
      console.error(`VIOLATION ${rel}`);
      violations.forEach((v) => console.error(`  ${v}`));
      totalViolations += violations.length;
    }
  }
}

if (FIXTURES_MODE) {
  if (silentFiles > 0) {
    console.error(`ambience boundary: ${silentFiles} fixture(s) produced NO violation — a boundary with no failing fixture is not enforced.`);
    process.exit(1);
  }
  console.log(`ambience boundary: ${scannedWithViolations} fixture(s) all violated as designed.`);
  process.exit(0);
}

if (totalViolations > 0) {
  console.error(`ambience boundary: ${totalViolations} violation(s) across ${files.length} file(s).`);
  process.exit(1);
}
console.log(`ambience boundary: ${files.length} file(s) clean.`);
