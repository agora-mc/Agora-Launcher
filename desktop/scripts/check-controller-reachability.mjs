#!/usr/bin/env node
/**
 * Fails on elements that can be clicked but never focused.
 *
 * This exists because of what an audit actually found: the primary Browse cards
 * and the version pickers in ModDetail were `<article>`, `<li>` and `<tr>`
 * elements carrying an `onClick`. A mouse could use them and nothing else
 * could — no keyboard, no screen reader, no controller. Counting `<button>`
 * elements hid the problem completely, because the buttons were all fine.
 *
 * The rule: a non-interactive element with an `onClick` must also be reachable,
 * which in practice means `tabIndex` plus a role and a key handler.
 *
 * Genuine exceptions exist — a modal backdrop is a click-outside affordance
 * rather than a control, and making it a tab stop would put a focus stop behind
 * a dialog. Those must say so:
 *
 *     // controller-exempt: backdrop, closing is also on the dialog's own button
 *     <div className="scrim" onClick={...}>
 *
 * The reason is required, and it is checked into the diff where a reviewer sees
 * it, rather than living in a list that drifts out of date.
 *
 * Usage:
 *   node scripts/check-controller-reachability.mjs
 *   node scripts/check-controller-reachability.mjs --fixtures --root <dir>
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
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
  : resolve(DESKTOP_ROOT, 'src');
const FIXTURES_MODE = args.includes('--fixtures');

/**
 * Tags with no built-in interactive semantics. `a` is absent on purpose: an
 * anchor without `href` is a styling choice this codebase does not make, and
 * one with `href` is already focusable.
 */
const NON_INTERACTIVE_TAGS = new Set([
  'div', 'span', 'li', 'ul', 'ol', 'tr', 'td', 'th', 'article', 'section',
  'aside', 'header', 'footer', 'nav', 'main', 'p', 'img', 'figure', 'label',
  'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
]);

const EXEMPTION = /controller-exempt:\s*\S/;

function sourceFiles(dir) {
  const found = [];
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules' || entry === 'dist') continue;
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      found.push(...sourceFiles(path));
      continue;
    }
    if (!/\.tsx$/.test(entry)) continue;
    if (!FIXTURES_MODE && /\.test\.tsx$/.test(entry)) continue;
    found.push(path);
  }
  return found;
}

function attributeNames(opening) {
  const names = new Set();
  for (const attribute of opening.attributes.properties) {
    if (ts.isJsxAttribute(attribute) && attribute.name) {
      names.add(attribute.name.getText());
    }
  }
  return names;
}

/**
 * True when an exemption comment sits just above the element or anywhere in its
 * own opening tag.
 *
 * Attributes are often spread over many lines here, and the natural place to
 * explain why something is pointer-only is next to the `onClick` itself. The
 * span deliberately stops at the end of the opening tag so a directive on a
 * nested child cannot exempt its parent.
 */
function isExempt(source, opening) {
  const lines = source.getFullText().split(/\r?\n/);
  const { line: startLine } = source.getLineAndCharacterOfPosition(opening.getStart(source));
  const { line: endLine } = source.getLineAndCharacterOfPosition(opening.getEnd());

  for (let line = Math.max(0, startLine - 3); line <= endLine; line += 1) {
    if (lines[line] && EXEMPTION.test(lines[line])) return true;
  }
  return false;
}

function checkFile(path) {
  const text = readFileSync(path, 'utf8');
  const source = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const violations = [];

  const visit = (node) => {
    const opening = ts.isJsxElement(node)
      ? node.openingElement
      : ts.isJsxSelfClosingElement(node)
        ? node
        : null;

    if (opening) {
      const tag = opening.tagName.getText();
      // Only intrinsic lowercase tags; a component decides its own semantics.
      if (NON_INTERACTIVE_TAGS.has(tag)) {
        const attributes = attributeNames(opening);
        const clickable = attributes.has('onClick');
        const focusable = attributes.has('tabIndex');
        if (clickable && !focusable && !isExempt(source, opening)) {
          const { line } = source.getLineAndCharacterOfPosition(opening.getStart(source));
          violations.push({
            file: relative(DESKTOP_ROOT, path).replace(/\\/g, '/'),
            line: line + 1,
            tag,
          });
        }
      }
    }

    ts.forEachChild(node, visit);
  };

  visit(source);
  return violations;
}

const files = sourceFiles(ROOT);
const violations = files.flatMap(checkFile);

if (FIXTURES_MODE) {
  const clean = files.filter((file) => checkFile(file).length === 0);
  if (clean.length > 0) {
    console.error('Controller reachability: these fixtures should have produced a violation:');
    for (const file of clean) console.error(`  ${relative(DESKTOP_ROOT, file)}`);
    process.exit(1);
  }
  console.log(`Controller reachability: all ${files.length} negative fixture(s) rejected.`);
  process.exit(0);
}

if (violations.length > 0) {
  console.error('Controller reachability: clickable elements that nothing but a mouse can reach.\n');
  for (const { file, line, tag } of violations) {
    console.error(`  ${file}:${line}  <${tag}> has onClick but no tabIndex`);
  }
  console.error('\nGive it tabIndex, a role and a key handler, or mark it:');
  console.error('  // controller-exempt: <why this is pointer-only>');
  process.exit(1);
}

console.log(`Controller reachability: OK (${files.length} files).`);
