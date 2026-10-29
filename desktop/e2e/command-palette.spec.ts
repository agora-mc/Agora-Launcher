import { test, expect, type Page } from '@playwright/test';

async function installPaletteMock(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const instances = [
      { instance_id: 'instance-one', name: 'Instance One', loader: 'fabric', loader_version: '0.16', minecraft_version: '1.21', is_locked: false, last_launched_at: null },
      { instance_id: 'instance-two', name: 'Instance Two', loader: 'neoforge', loader_version: '21', minecraft_version: '1.21', is_locked: false, last_launched_at: null },
    ];
    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'get_setting') {
          return Promise.resolve(args.key === 'onboarding_complete');
        }
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command === 'list_instances') return Promise.resolve(instances);
        if (command === 'get_instance_detail') {
          const row = instances.find((candidate) => candidate.instance_id === args.instanceId);
          return Promise.resolve({
            row,
            pack_gaps: [],
            manifest: {
              instance_id: row?.instance_id,
              name: row?.name,
              minecraft_version: row?.minecraft_version,
              loader: row?.loader,
              loader_version: row?.loader_version,
              is_locked: false,
              mods: [], resourcepacks: [], shaders: [], datapacks: [], worlds: [],
              user_preferences: {},
            },
          });
        }
        if (command.startsWith('list_')) return Promise.resolve([]);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  });
}

test.beforeEach(async ({ page }) => {
  await installPaletteMock(page);
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Home' })).toBeVisible();
});

test('shortcut and sidebar button open the command palette', async ({ page }) => {
  await page.getByRole('button', { name: 'Open command palette' }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.waitForTimeout(100); // allow Radix onOpenChange state to settle

  await page.evaluate(() => document.body.dispatchEvent(new KeyboardEvent('keydown', {
    key: 'k', ctrlKey: true, bubbles: true,
  })));
  await expect(page.getByRole('dialog')).toBeVisible();
});

test('keyboard selection skips headings and opens the exact instance', async ({ page }) => {
  await page.getByRole('button', { name: 'Open command palette' }).click();
  await expect(page.getByRole('option', { name: /Instance One/ })).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('ArrowDown');
  await expect(page.getByRole('option', { name: /Instance Two/ })).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('ArrowUp');
  await page.keyboard.press('Enter');

  await expect(page.getByRole('heading', { name: /Instance One/ })).toBeVisible();
});

test('no-result query keeps keyboard navigation safe', async ({ page }) => {
  await page.getByRole('button', { name: 'Open command palette' }).click();
  await page.getByPlaceholder('Type a command or search…').fill('no-such-result');
  await expect(page.getByText('No results found.')).toBeVisible();
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('ArrowUp');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog')).toBeVisible();
});
