#!/usr/bin/env node
/**
 * Dev-server launcher.
 *
 * `src/lib/site.ts` throws when neither NEXT_PUBLIC_GITHUB_REPOSITORY nor
 * GITHUB_REPOSITORY is set — correct for a release build, but it meant a bare
 * `npm run dev` crashed on the first render with no way to recover short of
 * exporting the variable by hand every session. CI and release builds still
 * pass the real slug (see docs/DEVELOPMENT.md); this only fills in the
 * upstream default when nothing else has, so local dev just works.
 *
 * An explicitly-set value always wins.
 */

import { spawn } from 'node:child_process';

const env = { ...process.env };
if (!env.NEXT_PUBLIC_GITHUB_REPOSITORY && !env.GITHUB_REPOSITORY) {
  env.NEXT_PUBLIC_GITHUB_REPOSITORY = 'agora-mc/Agora-Launcher';
  console.log(
    `[dev] NEXT_PUBLIC_GITHUB_REPOSITORY unset — defaulting to ${env.NEXT_PUBLIC_GITHUB_REPOSITORY}`,
  );
}

const child = spawn('npx', ['next', 'dev', ...process.argv.slice(2)], {
  stdio: 'inherit',
  shell: true,
  env,
});

child.on('exit', (code) => process.exit(code ?? 0));
