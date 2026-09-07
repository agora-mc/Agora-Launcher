import { test, expect } from '@playwright/test';

test('missing registry stays recoverable and retry success opens Browse', async ({ page }) => {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    let syncAttempts = 0;
    const missing = {
      has_cached_db: false, cached_tag: null, cached_schema_version: null,
      latest_tag: null, update_available: false, checked: true,
      message: 'No catalog found.',
    };
    const ready = {
      has_cached_db: true, cached_tag: 'test', cached_schema_version: 5,
      latest_tag: 'test', update_available: false, checked: true,
      message: 'Catalog ready.',
    };
    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'get_setting') return Promise.resolve(args.key === 'onboarding_complete');
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command === 'get_registry_status') return Promise.resolve(missing);
        if (command === 'check_registry_update') {
          syncAttempts += 1;
          return syncAttempts === 1
            ? Promise.reject(new Error('Network unavailable'))
            : Promise.resolve(ready);
        }
        if (command === 'list_instances') return Promise.resolve([]);
        if (command === 'list_snapshots') return Promise.resolve([]);
        if (command === 'list_categories' || command === 'list_manifest_loaders' || command === 'list_manifest_mc_versions') return Promise.resolve([]);
        if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'Browse', exact: true }).click();
  await expect(page.getByText('No catalog found.')).toBeVisible();
  await expect(page.getByText('No items to display.')).toHaveCount(0);

  await page.getByRole('button', { name: 'Retry' }).click();
  await expect(page.getByText(/Network unavailable/)).toBeVisible();
  await page.getByRole('button', { name: 'Retry' }).click();

  await expect(page.getByText('No items to display.')).toBeVisible();
  await expect(page.getByText(/Network unavailable/)).toHaveCount(0);
});
