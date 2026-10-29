import { test, expect, type Page } from '@playwright/test';

// The no-internet story, as the user sees it. Every failure here is mocked the
// way the backend reports it: a structured error with the code the core uses
// for "could not reach the server", or a connectivity snapshot that says so.

const NO_INTERNET_CATALOG =
  "Couldn't download the catalog: no internet connection.";
const NO_INTERNET_LOADER =
  "Couldn't download Fabric 0.20.1: no internet connection.";

interface OfflineMockOptions {
  connectivity: 'unknown' | 'reachable' | 'unreachable';
}

async function installOfflineMock(page: Page, options: OfflineMockOptions) {
  await page.addInitScript(({ options }) => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const missingCatalog = {
      has_cached_db: false,
      cached_tag: null,
      cached_schema_version: null,
      latest_tag: null,
      update_available: false,
      checked: false,
      message: 'No catalog found.',
    };
    const networkError = (message: string) => ({
      code: 'ERR_NETWORK_OFFLINE',
      message,
      details: null,
      suggested_action: 'Check your internet connection or proxy settings, then try again.',
    });
    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'advanced_mode') return Promise.resolve('true');
          if (key === 'modrinth_enabled') return Promise.resolve(false);
          return Promise.resolve(false);
        }
        if (command === 'set_setting') return Promise.resolve(null);
        if (command === 'get_connectivity_status') {
          return Promise.resolve({
            state: options.connectivity,
            last_outcome_at: options.connectivity === 'unknown' ? null : 1_700_000_000,
          });
        }
        if (command === 'get_registry_status') return Promise.resolve(missingCatalog);
        if (command === 'check_registry_update') {
          return Promise.reject(networkError(
            "Couldn't download the catalog: no internet connection.",
          ));
        }
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'get_lkg_marker') return Promise.resolve(null);
        if (command === 'list_instances') return Promise.resolve([]);
        if (command === 'list_snapshots') return Promise.resolve([]);
        if (command === 'list_categories') return Promise.resolve([]);
        if (command === 'list_instance_templates') return Promise.resolve([]);
        if (command === 'list_manifest_loaders') return Promise.resolve(['fabric', 'forge', 'quilt']);
        if (command === 'list_manifest_mc_versions') return Promise.resolve(['1.21']);
        if (command === 'list_loader_versions') {
          return Promise.resolve([
            { loader: 'fabric', mc_version: '1.21', loader_version: '0.20.1', file_type: 'stable' },
          ]);
        }
        if (command === 'create_instance') {
          return Promise.reject(networkError(
            "Couldn't download Fabric 0.20.1: no internet connection.",
          ));
        }
        if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
        if (command === 'for_you_items') return Promise.resolve([]);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  }, { options });
}

test.describe('one honest connectivity story', () => {
  test('Privacy says it cannot reach the internet when the last request failed', async ({ page }) => {
    await installOfflineMock(page, { connectivity: 'unreachable' });
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('tab', { name: 'Privacy', exact: true }).click();

    await expect(page.getByText("Can't reach the internet", { exact: true })).toBeVisible();
    await expect(page.getByText('Online', { exact: true })).toHaveCount(0);
    await expect(page.getByText(/last request could not connect/)).toBeVisible();
  });

  test('Privacy says Online only after a request got an answer', async ({ page }) => {
    await installOfflineMock(page, { connectivity: 'reachable' });
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('tab', { name: 'Privacy', exact: true }).click();

    await expect(page.getByText('Online', { exact: true })).toBeVisible();
    await expect(page.getByText("Can't reach the internet", { exact: true })).toHaveCount(0);
  });

  test('Privacy does not claim Online before any request has been made', async ({ page }) => {
    await installOfflineMock(page, { connectivity: 'unknown' });
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('tab', { name: 'Privacy', exact: true }).click();

    await expect(page.getByText('No requests yet', { exact: true })).toBeVisible();
    await expect(page.getByText('Online', { exact: true })).toHaveCount(0);
  });

  test('the offline banner appears when the app cannot reach the internet', async ({ page }) => {
    await installOfflineMock(page, { connectivity: 'unreachable' });
    await page.goto('/');
    await expect(page.getByText(/Can't reach the internet right now/)).toBeVisible();
    await expect(page.getByText(/You're offline/)).toHaveCount(0);
  });
});

test.describe('catalog and create failures say why', () => {
  test('Browse explains a missing catalog that could not be downloaded', async ({ page }) => {
    await installOfflineMock(page, { connectivity: 'unreachable' });
    await page.goto('/');
    await page.getByRole('button', { name: 'Browse', exact: true }).click();
    await expect(page.getByText('No catalog found.')).toBeVisible();

    await page.getByRole('button', { name: 'Retry' }).click();
    await expect(page.getByText(NO_INTERNET_CATALOG, { exact: true })).toBeVisible();
    await expect(page.getByText(/Retry downloads it again once you are online/)).toBeVisible();
    await expect(page.getByText(/You're offline/)).toHaveCount(0);
    await expect(page.getByText(/cached data/)).toHaveCount(0);
  });

  test('Create Instance names what failed and keeps Create available', async ({ page }) => {
    await installOfflineMock(page, { connectivity: 'unreachable' });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Create Instance' }).click();

    const dialog = page.getByRole('dialog');
    await expect(dialog.getByText('Create Custom Instance')).toBeVisible();
    // Long forms scroll inside the dialog instead of running off the top of the window.
    await expect(dialog).toHaveCSS('overflow-y', 'auto');

    // Loaders are shown with player-facing names; the values stay the ids.
    await expect(dialog.getByRole('option', { name: 'Fabric', exact: true })).toHaveCount(1);
    await expect(dialog.getByRole('option', { name: 'fabric', exact: true })).toHaveCount(0);

    await dialog.getByPlaceholder('e.g. Optimized Survival').fill('Offline Test');
    await dialog.getByRole('button', { name: 'Create', exact: true }).click();

    await expect(dialog.getByText(NO_INTERNET_LOADER, { exact: true })).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Create', exact: true })).toBeEnabled();
  });

  test('the Show advanced settings checkbox has an accessible name', async ({ page }) => {
    await installOfflineMock(page, { connectivity: 'reachable' });
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await expect(page.getByRole('checkbox', { name: 'Show advanced settings' })).toBeVisible();
  });
});
