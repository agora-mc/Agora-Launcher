import { test, expect, type Page } from '@playwright/test';

// The Community Governance page is now a static informational page. It makes
// no governance Tauri calls, so only the app bootstrap mock is required.

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

async function openGovernance(page: Page) {
  await installBootstrapMock(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Community Governance' }).click();
}

test('renders the governance intro and encourages voting by quality', async ({ page }) => {
  await openGovernance(page);

  await expect(page.getByRole('heading', { name: 'Community Governance' })).toBeVisible();
  await expect(page.getByText('Your votes shape the curated list — by quality, not by download count.')).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Why your vote matters' })).toBeVisible();
  await expect(page.getByText(/Rank by quality/)).toBeVisible();
  await expect(page.getByText(/not by the quantity of downloads/)).toBeVisible();
  await expect(page.getByText(/keeps the registry honest for everyone/)).toBeVisible();
});

test('explains how to vote in the app and on GitHub', async ({ page }) => {
  await openGovernance(page);

  await expect(page.getByRole('heading', { name: 'How to vote' })).toBeVisible();
  await expect(page.getByText('In the Agora app', { exact: true })).toBeVisible();
  await expect(page.getByText('On GitHub', { exact: true })).toBeVisible();
  await expect(page.getByText(/Open Settings → Accounts and sign in with GitHub/)).toBeVisible();
  await expect(page.getByText(/React with \+1 \(👍\) to upvote or -1 \(👎\) to downvote/)).toBeVisible();
  await expect(page.getByText(/Only direct reactions on the canonical vote issue count/)).toBeVisible();
});

test('shows Agora rules of engagement', async ({ page }) => {
  await openGovernance(page);

  await expect(page.getByRole('heading', { name: 'Agora’s rules of engagement' })).toBeVisible();
  for (const rule of ['Stay technical', 'No noise', 'Leave drama at the door', 'Be respectful', 'Zero tolerance']) {
    await expect(page.getByText(rule)).toBeVisible();
  }
  await expect(page.getByText(/immediate and permanent removal/)).toBeVisible();
});

test('links to Discord and GitHub', async ({ page }) => {
  await openGovernance(page);

  await expect(page.getByRole('heading', { name: 'Join the community' })).toBeVisible();
  const discord = page.getByRole('link', { name: 'Join the Discord' });
  await expect(discord).toBeVisible();
  await expect(discord).toHaveAttribute('href', /discord\.gg\//);
  await expect(discord).toHaveAttribute('target', '_blank');

  const github = page.getByRole('link', { name: 'GitHub Repository' });
  await expect(github).toBeVisible();
  await expect(github).toHaveAttribute('href', /github\.com\//);
  await expect(github).toHaveAttribute('target', '_blank');
});
