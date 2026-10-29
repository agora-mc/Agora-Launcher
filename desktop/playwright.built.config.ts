import { defineConfig } from '@playwright/test';
import base, { e2ePort } from './playwright.config';

// The same suite against the built bundle (`dist/`, what the desktop app
// ships), served as static files. Stable under a full parallel run, where the
// dev server's on-demand compiling caused bursts of refused connections, and it
// catches differences that only exist in the production build.
// Run with `npm run test:e2e:built`, which builds first.
export default defineConfig({
  ...base,
  webServer: {
    command: `npx vite preview --port ${e2ePort} --strictPort`,
    url: `http://localhost:${e2ePort}`,
    // A reused server could be serving an older build.
    reuseExistingServer: false,
    timeout: 60_000,
  },
});
