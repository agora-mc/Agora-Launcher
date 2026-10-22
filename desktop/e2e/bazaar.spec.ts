import { test, expect, type Page } from '@playwright/test';

type BrowseItem = Record<string, unknown>;

const item = (id: string, name: string, contentType = 'mod'): BrowseItem => ({
  id,
  source: 'curated',
  registryItem: {
    id,
    name,
    content_type: contentType,
    download_strategy: 'github_release',
    upvotes: 0,
    downvotes: 0,
    net_score: 0,
  },
  modrinthResult: null,
  name,
  iconUrl: null,
  description: 'A test mod.',
  contentType,
  heroImageUrl: null,
  author: 'tester',
  categories: ['optimization'],
  downloads: null,
  follows: null,
  upvotes: 0,
  downvotes: 0,
  netScore: 0,
  supportedVersions: ['1.21.8'],
  sourcePageUrl: null,
});

async function installBazaarMock(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      'agora-interaction-preference',
      JSON.stringify({ version: 1, value: 'high-interaction' }),
    );
    const calls: Array<{
      command: string;
      args: Record<string, unknown>;
      resolve: (value: unknown) => void;
      reject: (reason?: unknown) => void;
    }> = [];
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'get_setting') {
          const key = args.key;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'modrinth_enabled') return Promise.resolve(true);
          if (key === 'ambience.enabled') return Promise.resolve(true);
          return Promise.resolve(false);
        }
        if (command === 'get_registry_status') {
          return Promise.resolve({
            has_cached_db: true,
            cached_tag: 'test',
            cached_schema_version: 5,
            latest_tag: 'test',
            update_available: false,
            checked: true,
            message: 'Catalog ready.',
          });
        }
        if (command === 'list_categories') return Promise.resolve([]);
        if (command === 'list_provider_categories') return Promise.resolve([]);
        if (command === 'list_manifest_loaders' || command === 'list_manifest_mc_versions') return Promise.resolve([]);
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'list_instances') return Promise.resolve([]);
        if (command === 'list_snapshots') return Promise.resolve([]);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command === 'browse_search' || command === 'browse_load_more') {
          return new Promise((resolve, reject) => calls.push({ command, args, resolve, reject }));
        }
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __browseCalls: calls,
      __resolveBrowse(index: number, value: unknown) { calls[index].resolve(value); },
    });
  });
}

async function openBazaar(page: Page) {
  await page.goto('/');
  await page.getByRole('button', { name: 'Browse', exact: true }).click();
  await expect(page.getByTestId('browse-bazaar')).toBeVisible();
}

// Regression: the Bazaar's visually hidden live region sat 1px below the app
// shell, so the WINDOW gained a vertical scrollbar. Headless Chromium hides
// scrollbars, so this asserts the scroll size directly; the horizontal bar the
// user saw only appeared once the vertical one narrowed the window.
test('the Bazaar does not give the whole window a scrollbar', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await installBazaarMock(page);
  await openBazaar(page);
  // React StrictMode runs mount effects twice: resolve the newest search.
  await expect.poll(() => page.evaluate(() => (window as any).__browseCalls.length)).toBeGreaterThanOrEqual(2);
  const items = Array.from({ length: 24 }, (_, i) => item(`mod-${i}`, `Mod ${i}`));
  await page.evaluate((payload) => {
    (window as any).__resolveBrowse((window as any).__browseCalls.length - 1, { items: payload, total: 24, page: 0, hasMore: false });
  }, items);
  await expect(page.getByText('Mod 0', { exact: true }).first()).toBeVisible();
  await page.waitForTimeout(300);

  const doc = await page.evaluate(() => ({
    scrollHeight: document.documentElement.scrollHeight,
    clientHeight: document.documentElement.clientHeight,
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
  }));
  expect(doc.scrollHeight).toBeLessThanOrEqual(doc.clientHeight);
  expect(doc.scrollWidth).toBeLessThanOrEqual(doc.clientWidth);
});
