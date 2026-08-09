import { test, expect, type Page } from '@playwright/test';

// The About Agora page is a static informational page. Only the app
// bootstrap mock is required.

async function installBootstrapMock(page: Page) {
  await page.addInitScript(() => {
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: {
        transformCallback() { return 1; },
        unregisterCallback() {},
        invoke(command: string, args: Record<string, unknown> = {}) {
          if (command === 'get_setting') {
            const key = args.key as string;
            if (key === 'onboarding_complete') return Promise.resolve(true);
            if (key === 'ai_chat_enabled') return Promise.resolve(false);
            if (key === 'launch_mode') return Promise.resolve('delegation');
            if (key === 'last_home_visit') return Promise.resolve(null);
            return Promise.resolve(null);
          }
          if (command === 'set_setting') return Promise.resolve(null);
          if (command === 'get_registry_status' || command === 'check_registry_update') {
            return Promise.resolve({
              has_cached_db: true,
              cached_tag: 'test',
              cached_schema_version: 5,
              latest_tag: 'test',
              update_available: false,
              checked: true,
              message: 'Registry ready.',
            });
          }
          if (command === 'list_categories') return Promise.resolve([]);
          if (command === 'list_manifest_loaders') return Promise.resolve([]);
          if (command === 'list_manifest_mc_versions') return Promise.resolve([]);
          if (command === 'list_instances') return Promise.resolve([]);
          if (command === 'list_snapshots') return Promise.resolve([]);
          if (command === 'for_you_items') return Promise.resolve([]);
          if (command === 'get_windows_accent_color') return Promise.resolve(null);
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);
          return Promise.resolve(null);
        },
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  });
}

async function openAbout(page: Page) {
  await installBootstrapMock(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'The Agora Difference' }).click();
}

test('opens the About page from the sidebar', async ({ page }) => {
  await openAbout(page);

  await expect(page.getByRole('heading', { name: 'The Agora Difference' })).toBeVisible();
});

test('describes Agora as a bespoke boutique and covers its pillars', async ({ page }) => {
  await openAbout(page);

  await expect(page.getByText(/bespoke boutique, not a warehouse/)).toBeVisible();
  await expect(page.getByText(/tailored, and made accessible for you/)).toBeVisible();
  await expect(page.getByRole('heading', { name: 'What makes Agora unique' })).toBeVisible();
  for (const pillar of [
    'Customizable to you',
    'Democratic community voting',
    'Open source',
    'Free and ad-free',
    'Transparent',
    'Donations, not for-profit',
    'Autonomous and decentralized',
  ]) {
    await expect(page.getByRole('heading', { name: pillar })).toBeVisible();
  }
  await expect(page.getByRole('heading', { name: 'Tailored and accessible for you' })).toBeVisible();
});

test('links to GitHub and Discord', async ({ page }) => {
  await openAbout(page);

  const github = page.getByRole('link', { name: 'GitHub Repository' });
  await expect(github).toBeVisible();
  await expect(github).toHaveAttribute('href', /github\.com\//);
  await expect(github).toHaveAttribute('target', '_blank');

  const discord = page.getByRole('link', { name: 'Join the Discord' });
  await expect(discord).toBeVisible();
  await expect(discord).toHaveAttribute('href', /discord\.gg\//);
  await expect(discord).toHaveAttribute('target', '_blank');
});
