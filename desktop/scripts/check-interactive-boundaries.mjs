#!/usr/bin/env node
/**
 * Automated import-boundary check for the interactive experiences feature.
 *
 * Sol-1 contract: `docs/interactive/ARCHITECTURE_REVIEW_SOL.md` BLOCKER 1 and
 * `docs/interactive/SAFETY_BOUNDARIES.md` §2-3. This is an ALLOWLIST (fail
 * closed), not a denylist:
 *
 *   - each production area (`domain/`, `visual/`, `lab/`) may import only its
 *     documented internal layers plus a narrow set of framework packages;
 *   - local specifiers (relative, `@/` alias, explicit extensions, dynamic
 *     import, re-export, import-equals) are RESOLVED with the TypeScript
 *     compiler resolver before deciding whether an edge is allowed;
 *   - unknown or unresolvable local imports are failures;
 *   - `live/` is the only designated app-boundary layer (none present yet);
 *   - test files get additional test-framework allowances only;
 *   - shared visual components may not expose operation-like callback props
 *     (e.g. `onReview`) outside the declared `VisualIntent` union.
 *
 * Usage:
 *   node scripts/check-interactive-boundaries.mjs                 # normal mode
 *   node scripts/check-interactive-boundaries.mjs --root <dir> --fixtures
 *       # fixture mode: every scanned file must produce >=1 violation
 *
 * Exits non-zero with a report on any violation. Run via `npm run check:boundaries`
 * and as part of `npm run build`.
 */

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const ts = require('typescript');

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const DESKTOP_ROOT = resolve(SCRIPT_DIR, '..');
const INTERACTIVE_ROOT = resolve(DESKTOP_ROOT, 'src', 'features', 'interactive');

const args = process.argv.slice(2);
const rootIndex = args.indexOf('--root');
const ROOT = rootIndex >= 0 ? resolve(process.cwd(), args[rootIndex + 1]) : INTERACTIVE_ROOT;
const FIXTURES_MODE = args.includes('--fixtures');

// --- TypeScript config for module resolution (handles `@/*` alias + extensions) ---
const tsconfigPath = resolve(DESKTOP_ROOT, 'tsconfig.json');
const configFile = ts.readConfigFile(tsconfigPath, ts.sys.readFile);
if (configFile.error) {
  console.error('Interactive import-boundary check: cannot read tsconfig.json');
  process.exit(1);
}
const parsedConfig = ts.parseJsonConfigFileContent(configFile.config, ts.sys, DESKTOP_ROOT);
const compilerOptions = parsedConfig.options;

// --- Area classification ---
function classify(file) {
  const rel = file.replace(/\\/g, '/');
  if (rel.includes('/domain/')) return 'domain';
  if (rel.includes('/visual/')) return 'visual';
  if (rel.includes('/lab/')) return 'lab';
  if (rel.includes('/live/')) return 'live';
  if (rel.includes('/testing/')) return 'testing';
  return 'other';
}

function isTestFile(file) {
  return /\.(test|spec)\.(ts|tsx)$/.test(file);
}

/**
 * Allowed import roots per production area.
 * `internal`: interactive areas this area may import from.
 * `external`: framework packages (exact or prefix).
 */
const ALLOWED = {
  domain: { internal: ['domain'], external: [] },
  visual: { internal: ['domain', 'visual'], external: ['react', 'react/'] },
  lab: {
    internal: ['domain', 'visual', 'lab'],
    external: ['react', 'react/', '@radix-ui/react-dialog', '@radix-ui/react-dialog/'],
  },
  testing: { internal: ['domain', 'visual', 'lab', 'testing'], external: [] },
  live: null, // designated app boundary; not present in the current slice
  other: null,
};

const TEST_EXTERNAL = [
  'react',
  'react/',
  'vitest',
  '@testing-library/',
  'node:',
];

function isExternalAllowed(area, specifier, isTest) {
  const allow = isTest ? TEST_EXTERNAL : ALLOWED[area]?.external;
  if (!allow) return true;
  return allow.some((prefix) => specifier === prefix || specifier.startsWith(prefix));
}

/** Resolve a local specifier to an absolute file path, or null if external/unresolved. */
function resolveSpecifier(specifier, containingFile) {
  if (/^[./]/.test(specifier) || specifier.startsWith('@/')) {
    const result = ts.resolveModuleName(specifier, containingFile, compilerOptions, ts.sys);
    return result.resolvedModule?.resolvedFileName ?? null;
  }
  return null; // external package (not aliased)
}

/** Collect every module specifier: static import, re-export, import-equals, dynamic import. */
function collectSpecifiers(source) {
  const sf = ts.createSourceFile(
    'boundary.tsx',
    source,
    ts.ScriptTarget.Latest,
    /* setParentNodes */ false,
    ts.ScriptKind.TSX,
  );
  const specs = [];
  const visit = (node) => {
    if (ts.isImportDeclaration(node) && node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)) {
      specs.push(node.moduleSpecifier.text);
    } else if (ts.isExportDeclaration(node) && node.moduleSpecifier && ts.isStringLiteral(node.moduleSpecifier)) {
      specs.push(node.moduleSpecifier.text);
    } else if (
      ts.isImportEqualsDeclaration(node)
      && node.moduleReference
      && ts.isExternalModuleReference(node.moduleReference)
      && ts.isStringLiteral(node.moduleReference.expression)
    ) {
      specs.push(node.moduleReference.expression.text);
    } else if (
      ts.isCallExpression(node)
      && node.arguments.length === 1
      && ts.isStringLiteral(node.arguments[0])
      && ((ts.isIdentifier(node.expression) && node.expression.text === 'import')
        || node.expression.kind === ts.SyntaxKind.ImportKeyword)
    ) {
      specs.push(node.arguments[0].text);
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return specs;
}

// --- Shared-visual callback allowlist (BLOCKER 4 structural guard) ---
const ALLOWED_VISUAL_CALLBACKS = new Set([
  'onIntent',
  'onSelect',
  'onChange',
  // standard DOM / React event handlers (never operation-like)
  'onClick', 'onDoubleClick', 'onContextMenu',
  'onMouseDown', 'onMouseUp', 'onMouseEnter', 'onMouseLeave', 'onMouseMove', 'onMouseOver', 'onMouseOut',
  'onPointerDown', 'onPointerUp', 'onPointerMove', 'onPointerEnter', 'onPointerLeave', 'onPointerOver', 'onPointerOut',
  'onKeyDown', 'onKeyUp', 'onFocus', 'onBlur', 'onScroll', 'onInput', 'onSubmit', 'onWheel',
  'onDragStart', 'onDrag', 'onDragEnd', 'onDragEnter', 'onDragLeave', 'onDragOver', 'onDrop',
  'onAnimationStart', 'onAnimationEnd', 'onAnimationIteration', 'onTransitionEnd',
  'onCopy', 'onCut', 'onPaste', 'onTouchStart', 'onTouchEnd', 'onTouchMove', 'onTouchCancel',
  'onLoad', 'onError', 'onResize', 'onToggle',
]);

function checkVisualCallbacks(source, reportFn) {
  const re = /\b(on[A-Z][A-Za-z0-9]*)\s*[:?]\s/g; // prop definition like `onReview?:`
  let match;
  while ((match = re.exec(source)) !== null) {
    const name = match[1];
    if (!ALLOWED_VISUAL_CALLBACKS.has(name)) {
      reportFn(name);
    }
    re.lastIndex = match.index + 1;
  }
}

// --- Scan ---
function collectFiles(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const stat = statSync(full);
    if (stat.isDirectory()) {
      collectFiles(full, out);
    } else if (/\.(ts|tsx)$/.test(entry)) {
      out.push(full);
    }
  }
  return out;
}

const violations = [];
const violatedFiles = new Set();

function report(absFile, display, message) {
  violations.push(`${display}: ${message}`);
  violatedFiles.add(absFile);
}

function checkFile(file) {
  const source = readFileSync(file, 'utf8');
  const area = classify(file);
  if (area === 'other' || area === 'live' || !ALLOWED[area]) return;
  const isTest = isTestFile(file);
  const allowedInternal = ALLOWED[area].internal;
  const rel = relative(DESKTOP_ROOT, file).replace(/\\/g, '/');

  // Normalize path separators: TS resolution returns forward slashes on Windows.
  const interactiveNorm = INTERACTIVE_ROOT.replace(/\\/g, '/');
  const rootNorm = ROOT.replace(/\\/g, '/');

  for (const spec of collectSpecifiers(source)) {
    const resolved = resolveSpecifier(spec, file);
    if (resolved) {
      const resolvedNorm = resolved.replace(/\\/g, '/');
      if (resolvedNorm.startsWith(interactiveNorm) || resolvedNorm.startsWith(rootNorm)) {
        const targetArea = classify(resolved);
        if (targetArea === 'live') {
          report(file, rel, `'${spec}' imports from live/ — forbidden`);
          continue;
        }
        if (targetArea === 'other') {
          report(file, rel, `'${spec}' resolves to an unclassified interactive file (${relative(DESKTOP_ROOT, resolved)})`);
          continue;
        }
        if (!allowedInternal.includes(targetArea)) {
          report(file, rel, `'${spec}' -> ${targetArea}/ is outside the allowed internal roots [${allowedInternal.join(', ')}]`);
        }
      } else {
        // resolved to a local module outside interactive (app layer)
        report(file, rel, `'${spec}' resolves outside interactive to ${relative(DESKTOP_ROOT, resolved)} — app-layer dependency`);
      }
    } else if (spec.startsWith('@/') || /^[./]/.test(spec)) {
      report(file, rel, `'${spec}' did not resolve — unknown local import`);
    } else if (!isExternalAllowed(area, spec, isTest)) {
      report(file, rel, `external '${spec}' is not in the allowed list for ${area}${isTest ? ' (test)' : ''}`);
    }
  }

  if (area === 'visual') {
    checkVisualCallbacks(source, (name) =>
      report(file, rel, `shared visual exposes operation-like callback '${name}'; emit a declared VisualIntent instead`),
    );
  }
}

const files = collectFiles(ROOT);
for (const file of files) {
  checkFile(file);
}

if (FIXTURES_MODE) {
  const unflagged = files.filter((file) => !violatedFiles.has(file));
  if (violations.length > 0 && unflagged.length === 0) {
    console.log(`Interactive import-boundary fixtures OK — every fixture file produced a violation (${violations.length} total).`);
    process.exit(0);
  }
  console.error('Interactive import-boundary fixtures FAILED:');
  for (const file of unflagged) {
    console.error(`  - no violation for ${relative(DESKTOP_ROOT, file)}`);
  }
  process.exit(1);
}

if (violations.length > 0) {
  console.error(`Interactive import-boundary check FAILED (${violations.length} violation(s)):`);
  for (const violation of violations) {
    console.error(`  - ${violation}`);
  }
  process.exit(1);
}

console.log(`Interactive import-boundary check OK (${files.length} file(s) under ${relative(process.cwd(), ROOT)}).`);

