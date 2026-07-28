import { test, expect, type Page } from '@playwright/test';

const SANDBOX_CONFIG: Record<string, unknown> = {
  repository: 'Agora/registry-dev',
  environment: 'sandbox',
  github_app_slug: null,
  development_registry: true,
};

const SANDBOX_ONLY: Record<string, unknown> = {
  repository: 'Agora/registry-staging',
  environment: 'sandbox',
  github_app_slug: null,
  development_registry: false,
};

const DEVREG_ONLY: Record<string, unknown> = {
  repository: 'Agora/registry-dev',
  environment: 'production',
  github_app_slug: null,
  development_registry: true,
};

async function installMock(page: Page, config: Record<string, unknown> | null) {
  await page.addInitScript(
    (params: { config: Record<string, unknown> | null }) => {
      const { config } = params;

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
          // App bootstrapping
          if (command === 'get_setting') {
            const key = args.key as string;
            if (key === 'onboarding_complete') return Promise.resolve(true);
            if (key === 'ai_chat_enabled') return Promise.resolve(false);
            if (key === 'launch_mode') return Promise.resolve('delegation');
            if (key === 'modrinth_enabled') return Promise.resolve(true);
            if (key === 'last_home_visit') return Promise.resolve(null);
            return Promise.resolve(null);
          }
          if (command === 'set_setting') return Promise.resolve(null);
          if (command === 'get_registry_status') return Promise.resolve({
            has_cached_db: true,
            cached_tag: 'test',
            cached_schema_version: 5,
            latest_tag: 'test',
            update_available: false,
            checked: true,
            message: 'Registry ready.',
          });
          if (command === 'check_registry_update') return Promise.resolve({
            has_cached_db: true,
            cached_tag: 'test',
            cached_schema_version: 5,
            latest_tag: 'test',
            update_available: false,
            checked: true,
            message: 'Registry ready.',
          });
          if (command === 'list_categories') return Promise.resolve([]);
          if (command === 'list_instances') return Promise.resolve([]);
          if (command === 'list_manifest_loaders') return Promise.resolve([]);
          if (command === 'list_manifest_mc_versions') return Promise.resolve([]);
          if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
          if (command === 'for_you_items') return Promise.resolve([]);
          if (command === 'get_governance_config') return Promise.resolve(config);
          if (command === 'get_windows_accent_color') return Promise.resolve(null);
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);
          return Promise.resolve(null);
        },
      };
      Object.assign(window as unknown as Record<string, unknown>, {
        __TAURI_INTERNALS__: internals,
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      });
    },
    { config },
  );
}

test.describe('SandboxBanner', () => {

  test('shows sandbox governance active and development registry when both are true', async ({ page }) => {
    await installMock(page, SANDBOX_CONFIG);
    await page.goto('/');
    await expect(page.getByText(/Sandbox governance active/)).toBeVisible();
    await expect(page.getByText('Development registry')).toBeVisible();
  });

  test('shows only sandbox governance when environment is sandbox but not development_registry', async ({ page }) => {
    await installMock(page, SANDBOX_ONLY);
    await page.goto('/');
    await expect(page.getByText(/Sandbox governance active/)).toBeVisible();
    await expect(page.getByText('Development registry')).toHaveCount(0);
  });

  test('shows only development registry when production environment but development_registry true', async ({ page }) => {
    await installMock(page, DEVREG_ONLY);
    await page.goto('/');
    await expect(page.getByText('Development registry').first()).toBeVisible();
    await expect(page.getByText('Agora/registry-dev')).toBeVisible();
  });

  test('does not show banner when config is null (no backend support)', async ({ page }) => {
    await installMock(page, null);
    await page.goto('/');
    await expect(page.getByText(/Sandbox/)).toHaveCount(0);
    await expect(page.getByText(/Development/)).toHaveCount(0);
  });

  test('does not show banner for production registry with no sandbox', async ({ page }) => {
    await installMock(page, {
      repository: 'Agora/registry',
      environment: 'production',
      github_app_slug: null,
      development_registry: false,
    });
    await page.goto('/');
    await expect(page.getByText(/Sandbox/)).toHaveCount(0);
    await expect(page.getByText(/Development/)).toHaveCount(0);
  });

  test('uses ASCII hyphen not em-dash', async ({ page }) => {
    await installMock(page, SANDBOX_CONFIG);
    await page.goto('/');
    const bannerText = await page.getByRole('status').textContent();
    expect(bannerText).not.toContain('\u2014');
    expect(bannerText).toContain('Sandbox governance active -');
  });

  test('persistent banner re-appears on navigation', async ({ page }) => {
    await installMock(page, SANDBOX_CONFIG);
    await page.goto('/');
    await expect(page.getByText(/Sandbox governance active/)).toBeVisible();
    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByText(/Sandbox governance active/)).toBeVisible();
    await page.getByRole('button', { name: 'Home' }).click();
    await expect(page.getByText(/Sandbox governance active/)).toBeVisible();
  });

});
