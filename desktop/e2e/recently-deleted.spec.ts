import { test, expect, type Page } from '@playwright/test';

/**
 * Delete is recoverable: the confirm dialog says exactly what happens, the
 * deleted instance shows up under "Recently deleted", and Restore brings it
 * back (saying so when the id had been taken).
 */
async function installMock(page: Page, opts: { restoreRenamed: boolean }) {
  await page.addInitScript(({ opts }) => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const commandCalls: Record<string, number> = {};
    const lastCommandArgs: Record<string, Record<string, unknown>> = {};
    const instance = {
      instance_id: 'cf-pack',
      name: 'test',
      minecraft_version: '1.21',
      loader: 'fabric',
      loader_version: '0.16.9',
      is_locked: false,
      last_launched_at: null,
      import_source: 'curseforge',
    };
    let live: Record<string, unknown>[] = [instance];
    let deleted: Record<string, unknown>[] = [];
    const entry = {
      entry_id: '20260101T000000Z-abc',
      instance_id: 'cf-pack',
      name: 'test',
      minecraft_version: '1.21',
      loader: 'fabric',
      import_source: 'curseforge',
      deleted_at: '2026-01-01T00:00:00+00:00',
      expires_at: '2026-01-31T00:00:00+00:00',
      size_bytes: 5 * 1024 * 1024,
    };

    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        commandCalls[command] = (commandCalls[command] ?? 0) + 1;
        lastCommandArgs[command] = args;
        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'launch_mode') return Promise.resolve('direct');
          return Promise.resolve(false);
        }
        if (command === 'get_registry_status') {
          return Promise.resolve({
            has_cached_db: true, cached_tag: 'test', cached_schema_version: 5,
            latest_tag: 'test', update_available: false, checked: true, message: 'Catalog ready.',
          });
        }
        if (command === 'list_instances') return Promise.resolve(live);
        if (command === 'list_recently_deleted') return Promise.resolve(deleted);
        if (command === 'delete_instance') {
          live = [];
          deleted = [entry];
          return Promise.resolve(null);
        }
        if (command === 'restore_deleted_instance') {
          deleted = [];
          live = [instance];
          return Promise.resolve({
            instance_id: opts.restoreRenamed ? 'cf-pack-restored' : 'cf-pack',
            original_instance_id: 'cf-pack',
            renamed: opts.restoreRenamed,
          });
        }
        if (command === 'purge_deleted_instance') {
          deleted = [];
          return Promise.resolve(null);
        }
        if (command === 'list_instance_templates' || command === 'query_launch_state') return Promise.resolve([]);
        if (command === 'plugin:event|listen') return Promise.resolve(1);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __commandCalls: commandCalls,
      __lastCommandArgs: lastCommandArgs,
      __callbacks: callbacks,
    });
  }, { opts });
}

const calls = (page: Page, command: string) =>
  page.evaluate((c) => ((window as any).__commandCalls as Record<string, number>)[c] ?? 0, command);

test.describe('Recently deleted', () => {
  test('delete says what it does, lists the instance, and Restore brings it back', async ({ page }) => {
    await installMock(page, { restoreRenamed: false });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await expect(page.getByRole('heading', { name: 'test' })).toBeVisible();
    await expect(page.getByText('Recently deleted')).toHaveCount(0);

    await page.getByRole('button', { name: 'Delete', exact: true }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toContainText(
      "Removes Agora's copy of test. Your CurseForge files aren't touched. You can restore it from Recently deleted for 30 days.",
    );
    await expect(dialog).not.toContainText('trash');
    await dialog.getByRole('button', { name: 'Delete' }).click();

    const section = page.getByRole('region', { name: 'Recently deleted' });
    await expect(section).toBeVisible();
    await expect(section).toContainText('test');
    expect(await calls(page, 'delete_instance')).toBe(1);

    await section.getByRole('button', { name: 'Restore' }).click();
    await expect(page.getByRole('heading', { name: 'test' })).toBeVisible();
    await expect(page.getByText('Restored test.')).toBeVisible();
    expect(await calls(page, 'restore_deleted_instance')).toBe(1);
    const args = await page.evaluate(() => (window as any).__lastCommandArgs['restore_deleted_instance']);
    expect(args.entryId).toBe('20260101T000000Z-abc');
  });

  test('restore says so when the original name was taken', async ({ page }) => {
    await installMock(page, { restoreRenamed: true });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Delete', exact: true }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Delete' }).click();
    await page.getByRole('region', { name: 'Recently deleted' }).getByRole('button', { name: 'Restore' }).click();
    await expect(page.getByText(/Restored test as "test \(restored\)"/)).toBeVisible();
  });

  test('Delete now asks first and then removes the entry', async ({ page }) => {
    await installMock(page, { restoreRenamed: false });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Delete', exact: true }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Delete' }).click();
    const section = page.getByRole('region', { name: 'Recently deleted' });
    await section.getByRole('button', { name: 'Delete now' }).click();
    await expect(page.getByRole('dialog')).toContainText('Recycle Bin');
    expect(await calls(page, 'purge_deleted_instance')).toBe(0);
    await page.getByRole('dialog').getByRole('button', { name: 'Delete now' }).click();
    await expect(section).toHaveCount(0);
    expect(await calls(page, 'purge_deleted_instance')).toBe(1);
  });
});
