import { defineConfig, devices } from '@playwright/test';

// E2E_PORT lets a second run (an agent's worktree, another session) use its own
// port instead of colliding on 5173 or editing this file.
export const e2ePort = Number(process.env.E2E_PORT ?? 5173);
const baseURL = `http://localhost:${e2ePort}`;

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  // Line reporter keeps the CLI attached to stdout and never auto-opens a
  // browser; the HTML report is still written but only opened on demand via
  // `npx playwright show-report`. (Auto-open + webServer teardown on Windows
  // could leave the process alive after tests complete.)
  reporter: [['line'], ['html', { open: 'never' }]],
  use: {
    baseURL,
    trace: 'on-first-retry',
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
  ],
  // The Vite dev server: fast to iterate against, but it compiles modules on
  // request, so a full parallel run can overload it. Full runs use
  // `playwright.built.config.ts` (`npm run test:e2e:built`) instead.
  webServer: {
    command: `npm run dev -- --port ${e2ePort} --strictPort`,
    url: baseURL,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
