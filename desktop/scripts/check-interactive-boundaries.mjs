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
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const ts = require('typescript');

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const DESKTOP_ROOT = resolve(SCRIPT_DIR, '..');

const args = process.argv.slice(2);
const rootIndex = args.indexOf('--root');
const ROOT = rootIndex >= 0
  ? resolve(process.cwd(), args[rootIndex + 1])
  : resolve(DESKTOP_ROOT, 'src', 'features', 'interactive');
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

// --- Area classification (path-segment based, NOT string-prefix) ---
const AREA_NAMES = ['domain', 'visual', 'lab', 'live', 'testing'];

/** True when `file` is inside `root` (path-segment containment). */
function isWithin(root, file) {
  const rel = relative(root, file).replace(/\\/g, '/');
  return rel !== '' && !rel.startsWith('..') && !isAbsolute(rel);
}

function classify(file) {
  if (!isWithin(ROOT, file)) return 'other';
  const rel = relative(ROOT, file).replace(/\\/g, '/');
  const first = rel.split('/')[0];
  return AREA_NAMES.includes(first) ? first : 'other';
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

/**
 * Enforce the shared-visual callback allowlist with the TypeScript AST so BOTH
 * property signatures (`onReview?: () => void`) and method signatures
 * (`onReview(): void`) are rejected. Only type declarations (interfaces and
 * inline type literals) are inspected — JSX usage attributes are not.
 */
function checkVisualCallbacks(source, reportFn) {
  const sf = ts.createSourceFile('visual.tsx', source, ts.ScriptTarget.Latest, false, ts.ScriptKind.TSX);
  const checkMember = (member) => {
    if (!member.name) return;
    if (!ts.isIdentifier(member.name) && !ts.isStringLiteral(member.name)) return;
    const name = member.name.text;
    if (!/^on[A-Z]/.test(name)) return;
    if (ALLOWED_VISUAL_CALLBACKS.has(name)) return;
    const kind = ts.isMethodSignature(member) || ts.isMethodDeclaration(member) ? 'method' : 'property';
    reportFn(name, kind);
  };
  const visit = (node) => {
    if (ts.isInterfaceDeclaration(node)) {
      node.members.forEach(checkMember);
      return; // members handled; do not recurse into them
    }
    if (ts.isTypeLiteralNode(node)) {
      node.members.forEach(checkMember);
      return;
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
}

// --- Live subarea boundary (SOL-2 BLOCKER 5) ---
// The `live/` layer is the only app-boundary, but it is NOT a free-for-all:
// read adapters/loaders may invoke only read-only Tauri commands; the core
// host may import tauri types only; only named operation bridges may host the
// approved Standard controllers. Unknown live files and direct mutation
// imports outside those boundaries must fail the build.

/** Read-only Tauri commands the live READ layer may invoke. */
const TAURI_READ_COMMANDS = new Set([
  'listInstances',
  'getInstanceDetail',
  'listInstanceContent',
  'enrichInstanceContent',
  'getDependencyGraph',
  'queryLaunchState',
  'checkInstanceHealth',
  'checkAllInstanceHealth',
  'listLoaderVersions',
  'listSnapshots',
  'detectDrift',
  'listCrashReports',
  'readCrashLog',
  'checkInstanceCrash',
  'triageCrashReport',
  'investigateInstanceEvidence',
  'pickAndInvestigateCrashEvidence',
  'listJavaRuntimes',
  'recommendInstanceMemory',
  'getSetting',
  'getRegistryStatus',
  'listCategories',
  'getRegistryItem',
  'fetchModrinthProject',
  'isModrinthEnabled',
  'browseItems',
  'forYouItems',
]);

/** Live subarea classification (SOL-2 BLOCKER 5). */
function liveSubarea(file) {
  const norm = file.replace(/\\/g, '/');
  if (norm.includes('/live/readAdapters/')) return 'read';
  if (norm.endsWith('/live/liveScene.ts')) return 'read';
  if (norm.endsWith('/live/freshness.ts')) return 'read'; // read-pipeline helper (labels read results)
  if (norm.includes('/live/operationBridges/')) return 'bridges';
  if (norm.includes('/live/')) {
    const knownCore = [
      '/live/LiveInteractiveHost.tsx',
      '/live/LiveSceneView.tsx',
      '/live/liveCapabilities.ts',
      '/live/presentationPreference.ts',
      '/live/intentController.ts',
      // The v4-world foreground port (V5-PORT-PLAN §5 phase 4): presentation
      // only, emits VisualIntent via the host, no tauri.
      '/live/WorldEditor.tsx',
      '/live/worldEditorData.ts',
      // Persisted interaction achievements (pure localStorage, no tauri).
      '/live/interactionAchievements.ts',
    ];
    return knownCore.some((name) => norm.endsWith(name)) ? 'core' : 'unknown';
  }
  return null;
}

/**
 * Classify EVERY way a live file imports a specific specifier (SOL-2 BLOCKER C).
 *
 * Returns an ARRAY of `{ form, names }` — one entry per matching statement:
 *   - 'none'            — type-only (or entirely type-annotated): safe anywhere;
 *   - 'named'           — enumerable non-type ORIGINAL names (static import /
 *                         named re-export), evaluated as
 *                         `propertyName?.text ?? name.text` so an aliased
 *                         mutation (`restoreSnapshot as getInstanceDetail`) is
 *                         judged by its ORIGINAL name;
 *   - 'namespace'       — `import * as ns from` (names cannot be verified);
 *   - 'side-effect'     — `import '@/lib/tauri'` (no names at all);
 *   - 'dynamic'         — `import('@/lib/tauri')`;
 *   - 'reexport-star'   — `export * from` / `export * as ns from`;
 *   - 'import-equals'   — `import tauri = require('@/lib/tauri')`.
 *
 * Aggregation closes the last-write-wins hole: a prohibited value import that
 * appears anywhere in the file is never hidden by a later type-only or
 * allowlisted import of the same specifier. Everything except 'none'/'named'
 * is an UNVERIFIABLE value import and is rejected in the live layer — never
 * silently treated as type-only.
 */
function tauriImportForms(source, targetSpecifier) {
  const sf = ts.createSourceFile('live.tsx', source, ts.ScriptTarget.Latest, false, ts.ScriptKind.TSX);
  const forms = [];
  const visit = (node) => {
    if (
      ts.isImportDeclaration(node)
      && node.moduleSpecifier
      && ts.isStringLiteral(node.moduleSpecifier)
      && node.moduleSpecifier.text === targetSpecifier
    ) {
      const clause = node.importClause;
      if (!clause) { forms.push({ form: 'side-effect', names: [] }); return; }
      if (clause.isTypeOnly) { forms.push({ form: 'none', names: [] }); return; }
      const bindings = clause.namedBindings;
      if (!bindings) { forms.push({ form: 'side-effect', names: [] }); return; }
      if (ts.isNamedImports(bindings)) {
        const valueNames = bindings.elements
          .filter((element) => !element.isTypeOnly)
          // Original imported name, NOT the local binding (alias-safe).
          .map((element) => element.propertyName?.text ?? element.name.text);
        forms.push(valueNames.length === 0
          ? { form: 'none', names: [] }
          : { form: 'named', names: valueNames });
        return;
      }
      if (ts.isNamespaceImport(bindings)) { forms.push({ form: 'namespace', names: ['*'] }); return; }
    }
    if (
      ts.isExportDeclaration(node)
      && node.moduleSpecifier
      && ts.isStringLiteral(node.moduleSpecifier)
      && node.moduleSpecifier.text === targetSpecifier
    ) {
      if (node.isTypeOnly) { forms.push({ form: 'none', names: [] }); return; }
      const clause = node.exportClause;
      if (!clause || !ts.isNamedExports(clause)) { forms.push({ form: 'reexport-star', names: [] }); return; }
      const valueNames = clause.elements
        .filter((element) => !element.isTypeOnly)
        .map((element) => element.propertyName?.text ?? element.name.text);
      forms.push(valueNames.length === 0
        ? { form: 'none', names: [] }
        : { form: 'named', names: valueNames });
      return;
    }
    if (
      ts.isImportEqualsDeclaration(node)
      && node.moduleReference
      && ts.isExternalModuleReference(node.moduleReference)
      && ts.isStringLiteral(node.moduleReference.expression)
      && node.moduleReference.expression.text === targetSpecifier
    ) {
      forms.push({ form: 'import-equals', names: [] });
      return;
    }
    if (
      ts.isCallExpression(node)
      && node.arguments.length === 1
      && ts.isStringLiteral(node.arguments[0])
      && node.arguments[0].text === targetSpecifier
      && ((ts.isIdentifier(node.expression) && node.expression.text === 'import')
        || node.expression.kind === ts.SyntaxKind.ImportKeyword)
    ) {
      forms.push({ form: 'dynamic', names: [] });
      return;
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return forms;
}

function describeTauriForm(form) {
  switch (form) {
    case 'named': return 'named imports';
    case 'namespace': return 'a namespace import';
    case 'side-effect': return 'a side-effect import';
    case 'dynamic': return 'a dynamic import';
    case 'reexport-star': return 'a star re-export';
    case 'import-equals': return 'an import-equals';
    default: return 'an unverifiable import form';
  }
}

/** External packages a live subarea may import (narrow; no tauri, no app libs). */
const LIVE_EXTERNAL = ['react', 'react/'];

/** Allowed live subarea -> live subarea edges (SOL-2 BLOCKER C). */
const LIVE_EDGES = {
  read: new Set(['read']),
  core: new Set(['read', 'core', 'bridges']),
  bridges: new Set(['read', 'core', 'bridges']),
};

function checkLiveFile(file, source, rel, reportFn) {
  if (isTestFile(file)) return; // test files get test-framework allowances
  const subarea = liveSubarea(file);
  if (subarea === null) return; // not a live file
  if (subarea === 'unknown') {
    reportFn(`${rel}: unclassified live/ file — add it to a known live subarea (read, core, or operationBridges)`);
    return;
  }
  const allowedSubareas = LIVE_EDGES[subarea];

  for (const spec of collectSpecifiers(source)) {
    // Styles are side-effect imports; the TS resolver cannot resolve .css and
    // they carry no runtime dependency edge worth enforcing.
    if (spec.endsWith('.css')) continue;
    // Tauri is only reachable by the read layer via a narrow named import.
    if (spec === '@/lib/tauri' || spec.startsWith('@/lib/tauri/')) {
      const forms = tauriImportForms(source, spec);
      if (forms.length === 0) continue; // specifier appears only in other files
      let flagged = false;
      for (const form of forms) {
        if (form.form === 'none') continue; // type-only is allowed everywhere
        if (subarea === 'read') {
          if (form.form === 'named') {
            const bad = form.names.filter((name) => !TAURI_READ_COMMANDS.has(name));
            if (bad.length > 0) {
              flagged = true;
              reportFn(`${rel}: live read layer invokes non-read Tauri command(s): ${bad.join(', ')}`);
            }
          } else {
            flagged = true;
            reportFn(`${rel}: live read layer uses ${describeTauriForm(form.form)} from '${spec}' — command names cannot be verified read-only`);
          }
        } else {
          flagged = true;
          reportFn(`${rel}: live ${subarea} layer must not import '${spec}' at runtime (${describeTauriForm(form.form)}); type-only imports only`);
        }
      }
      if (flagged) continue; // a prohibited form was already reported for this spec
      continue;
    }

    const resolved = resolveSpecifier(spec, file);
    if (resolved) {
      if (isWithin(ROOT, resolved)) {
        const targetArea = classify(resolved);
        if (targetArea === 'live') {
          const targetSubarea = liveSubarea(resolved);
          if (targetSubarea === null || targetSubarea === 'unknown') {
            reportFn(`${rel}: '${spec}' resolves to an unclassified live/ file (${relative(DESKTOP_ROOT, resolved)})`);
          } else if (!allowedSubareas.has(targetSubarea)) {
            reportFn(`${rel}: live ${subarea} -> ${targetSubarea} edge '${spec}' is not allowed`);
          }
        } else if (targetArea !== 'domain' && targetArea !== 'visual') {
          reportFn(`${rel}: '${spec}' resolves to ${targetArea}/ which is not allowed for live ${subarea}`);
        }
      } else if (isWithin(DESKTOP_ROOT, resolved)) {
        // resolved to a local module outside the scan root (app layer, e.g.
        // lib/tauri via a relative path, or any other app module)
        reportFn(`${rel}: '${spec}' resolves outside interactive to ${relative(DESKTOP_ROOT, resolved)} — app-layer dependency`);
      } else if (!LIVE_EXTERNAL.some((prefix) => spec === prefix || spec.startsWith(prefix))) {
        reportFn(`${rel}: external '${spec}' is not allowed for live ${subarea}`);
      }
    } else if (spec.startsWith('@/') || /^[./]/.test(spec)) {
      reportFn(`${rel}: '${spec}' did not resolve — unknown local import`);
    } else if (!LIVE_EXTERNAL.some((prefix) => spec === prefix || spec.startsWith(prefix))) {
      reportFn(`${rel}: external '${spec}' is not allowed for live ${subarea}`);
    }
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
  const rel = relative(DESKTOP_ROOT, file).replace(/\\/g, '/');

  // `live/` is the designated app boundary, but it is enforced by subarea
  // (read/core/bridges) rather than exempted (SOL-2 BLOCKER 5).
  if (area === 'live') {
    checkLiveFile(file, source, rel, (message) => report(file, rel, message));
    return;
  }
  if (area === 'other') {
    report(file, rel, 'unclassified source under the interactive scan root; only live/ subareas are classified');
    return;
  }

  const isTest = isTestFile(file);
  const allowedInternal = ALLOWED[area].internal;

  for (const spec of collectSpecifiers(source)) {
    // Styles are side-effect imports; they carry no runtime dependency edge.
    if (spec.endsWith('.css')) continue;
    const resolved = resolveSpecifier(spec, file);
    if (resolved) {
      if (isWithin(ROOT, resolved)) {
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
        // resolved to a local module outside the scan root (app layer)
        report(file, rel, `'${spec}' resolves outside interactive to ${relative(DESKTOP_ROOT, resolved)} — app-layer dependency`);
      }
    } else if (spec.startsWith('@/') || /^[./]/.test(spec)) {
      report(file, rel, `'${spec}' did not resolve — unknown local import`);
    } else if (!isExternalAllowed(area, spec, isTest)) {
      report(file, rel, `external '${spec}' is not in the allowed list for ${area}${isTest ? ' (test)' : ''}`);
    }
  }

  if (area === 'visual') {
    checkVisualCallbacks(source, (name, kind) =>
      report(file, rel, `shared visual exposes operation-like ${kind} '${name}'; emit a declared VisualIntent instead`),
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

