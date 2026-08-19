import { test, expect } from '@playwright/test';

/**
 * Verify that one failed setting read does not prevent others from loading,
 * and that the page still renders fully.
 */
test('one failed setting does not cascade and settings page renders', async ({ page }) => {
  await page.addInitScript(() => {
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
          const key = args.key as string;
          // ai_mcp_enabled fails to load — simulate backend error.
          if (key === 'ai_mcp_enabled') {
            return Promise.reject(new Error('Backend unavailable'));
          }
          // All other settings succeed.
          if (key === 'modrinth_enabled') return Promise.resolve(true);
          if (key === 'always_pre_touch') return Promise.resolve(true);
            if (key === 'mojang_launcher_path') return Promise.resolve('');
          if (key === 'launch_mode') return Promise.resolve('delegation');
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'ai_chat_enabled') return Promise.resolve(true);
          return Promise.resolve(null);
        }
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'list_instances') return Promise.resolve([]);
        if (command === 'list_snapshots') return Promise.resolve([]);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command === 'set_setting') return Promise.resolve(null);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();

  // Every section is still reachable even though ai_mcp_enabled failed to
  // load. Sensible defaults are used for the failed setting.
  const sections = page.getByRole('navigation', { name: 'Settings sections' });
  await expect(sections.getByRole('tab', { name: 'Appearance' })).toBeVisible();

  await sections.getByRole('tab', { name: 'Services' }).click();
  await expect(page.getByText('Modrinth Integration', { exact: true })).toBeVisible();

  await sections.getByRole('tab', { name: 'Accounts' }).click();
  await expect(page.getByRole('heading', { name: 'GitHub Account' })).toBeVisible();

  await sections.getByRole('tab', { name: 'Launching' }).click();
  await expect(page.getByRole('heading', { name: 'Launch Mode' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Launcher Path' })).toBeVisible();
  await expect(page.locator('#settings-launching')).toContainText('Launch Mode');
  await expect(page.locator('#settings-launching')).toBeInViewport();
});

/**
 * The section rail is the whole navigation model now: a sub-page belongs to
 * exactly one section, and the section that is open survives leaving the page.
 */
test('settings tabs switch sub-pages and remember the open section', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = {
      transformCallback() { return 1; },
      unregisterCallback() {},
      invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'get_setting') return Promise.resolve((args.key as string) === 'onboarding_complete');
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'list_instances') return Promise.resolve([]);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();

  const sections = page.getByRole('navigation', { name: 'Settings sections' });
  await expect(sections.getByRole('tab', { name: 'General' })).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByRole('heading', { name: 'Advanced mode' })).toBeVisible();

  // Sub-pages of the open section swap the panel without touching the rail.
  await page.getByRole('tab', { name: 'Walkthrough' }).click();
  await expect(page.getByRole('heading', { name: 'Guided walkthrough' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Advanced mode' })).toBeHidden();

  await sections.getByRole('tab', { name: 'Appearance' }).click();
  await expect(page.getByLabel('Accent source')).toBeVisible();

  // Leaving Settings and coming back returns to the section that was open.
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(sections.getByRole('tab', { name: 'Appearance' })).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByLabel('Accent source')).toBeVisible();
});

/**
 * The Living Background page only appears in the sidebar while the world is
 * on, so Settings carries the signpost to it.
 */
test('appearance links out to the Living Background page', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = {
      transformCallback() { return 1; },
      unregisterCallback() {},
      invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'get_setting') return Promise.resolve((args.key as string) === 'onboarding_complete');
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'list_instances') return Promise.resolve([]);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('navigation', { name: 'Settings sections' })
    .getByRole('tab', { name: 'Appearance' }).click();
  await page.getByRole('tab', { name: 'Living background' }).click();

  const signpost = page.getByTestId('living-background-settings');
  await expect(signpost.getByText('Open the Living Background page')).toBeVisible();
  await signpost.getByRole('button', { name: /^(Open|Turn on and open)$/ }).click();

  await expect(page.getByTestId('living-background-page')).toBeVisible();
});

test('boolean settings are sent to Tauri as JSON booleans', async ({ page }) => {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    const writes: Array<{ key: string; value: unknown }> = [];
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
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'modrinth_enabled') return Promise.resolve(false);
          if (key === 'always_pre_touch') return Promise.resolve(true);
          if (key === 'launch_mode') return Promise.resolve('delegation');
          if (key === 'mojang_launcher_path') return Promise.resolve('');
          return Promise.resolve(null);
        }
        if (command === 'set_setting') {
          writes.push({ key: args.key, value: args.value });
          return Promise.resolve(null);
        }
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'list_instances') return Promise.resolve([]);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __settingWrites: writes,
    });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  // General > Basics is the landing section, so the install toggles are here.
  await expect(page.getByLabel('Always auto-confirm installs', { exact: true })).toBeDisabled();

  await page.getByLabel('Auto-confirm clean installs', { exact: true }).check();
  await expect.poll(async () => page.evaluate(() => (window as any).__settingWrites)).toContainEqual({
    key: 'install_auto_confirm_clean',
    value: true,
  });

  await page.getByLabel('Always auto-confirm installs', { exact: true }).check();
  await expect.poll(async () => page.evaluate(() => (window as any).__settingWrites)).toContainEqual({
    key: 'install_always_auto_confirm',
    value: true,
  });

  await page.getByRole('navigation', { name: 'Settings sections' })
    .getByRole('tab', { name: 'Services' }).click();
  await expect(page.getByText('Modrinth Integration', { exact: true })).toBeVisible();
  await page.locator('#settings-services input[type="checkbox"]').first().check();

  await expect.poll(async () => page.evaluate(() => (window as any).__settingWrites)).toContainEqual({
    key: 'modrinth_enabled',
    value: true,
  });
});

test('software updates section shows the packaged app version from Tauri', async ({ page }) => {
  await page.addInitScript(() => {
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
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          return Promise.resolve(null);
        }
        if (command === 'plugin:app|version') return Promise.resolve('9.8.7-test');
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'list_instances') return Promise.resolve([]);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('tab', { name: 'Updates' }).click();
  await expect(page.getByRole('heading', { name: 'Software Updates' })).toBeVisible();
  await expect(page.locator('#settings-updates')).toContainText('Agora Launcher 9.8.7-test');
});

test('open application data folder invokes the open_data_folder command', async ({ page }) => {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const calls: string[] = [];
    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'open_data_folder') {
          calls.push(command);
          return Promise.resolve();
        }
        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          return Promise.resolve(null);
        }
        if (command === 'plugin:app|version') return Promise.resolve('0.1.0');
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'list_instances') return Promise.resolve([]);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __dataFolderCalls: calls,
    });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('tab', { name: 'Updates' }).click();
  await page.getByRole('button', { name: 'Open application data folder' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__dataFolderCalls)).toEqual(['open_data_folder']);
});
