import { defineConfig, devices } from '@playwright/test';

/**
 * Scratch config (not committed): the main checkout owns port 5173 and vite
 * pins it with strictPort, so this worktree's dev server runs on 5199 and is
 * started outside Playwright. Same tests, different base URL.
 */
export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  reporter: [['line']],
  use: {
    baseURL: 'http://localhost:5199',
    trace: 'off',
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
  ],
});
