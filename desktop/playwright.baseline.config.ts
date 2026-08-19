import { defineConfig, devices } from '@playwright/test';
export default defineConfig({
  testDir: 'D:/Agora/desktop/e2e',
  fullyParallel: true,
  reporter: [['line']],
  use: { baseURL: 'http://localhost:5173', trace: 'off' },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
});
