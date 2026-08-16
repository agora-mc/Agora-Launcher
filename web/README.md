# Agora (Web)

Static Next.js website for the public curated mod directory.

## Setup

```bash
npm install
```

## Build

The website reads two generated artifacts at build time, plus the desktop app's
guide content directly from source.

1. Generate the registry artifact:
   ```bash
   python compiler/compile.py --skip-sign
   ```
   This produces both `registry.db` and `registry-web.json`.

2. Generate the documentation index:
   ```bash
   python scripts/build_docs_web.py
   ```
   This produces `docs-web.json` from the repository's markdown, including the
   audience/group classification the docs navigation is built from.

3. Build the static site:
   ```bash
   NEXT_PUBLIC_GITHUB_REPOSITORY=agora-mc/Agora-Launcher npm run build
   ```

This produces a static export in `dist/` that can be deployed to Vercel or GitHub Pages.

## Development

```bash
NEXT_PUBLIC_GITHUB_REPOSITORY=agora-mc/Agora-Launcher npm run dev
```

`NEXT_PUBLIC_GITHUB_REPOSITORY` (or `GITHUB_REPOSITORY`) is required — `src/lib/site.ts`
throws without it. Set `REGISTRY_WEB_JSON_PATH` or `DOCS_WEB_JSON_PATH` to override the
default `../registry-web.json` and `../docs-web.json` lookup paths.

## Guide content

`/docs/guides/` is rendered from `desktop/src/data/guideContent.ts`, imported directly
via the `@guide/*` path alias and Next's `experimental.externalDir`. The website keeps no
copy of that text, so the app and the site cannot describe the product differently. Add or
edit guide topics in the desktop file, not here.
