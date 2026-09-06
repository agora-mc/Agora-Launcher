import { test, expect, type Page } from '@playwright/test';

/**
 * Shared mock for both invalid-mod-id and invalid-instance-id tests.
 * Provides sufficient command coverage for the ModDetail and InstanceEditor
 * page components to mount without unhandled invoke rejections.
 *
 * Key mocked commands by test variant:
 * - invalid-mod:   get_registry_item → null, fetch_modrinth_project → null
 * - invalid-instance: get_instance_detail → null
 */
async function installMock(page: Page, options: { variant: 'invalid-mod' | 'invalid-instance' }) {
  await page.addInitScript(({ options }) => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;

    const instances = [
      { instance_id: 'my-instance', name: 'My Instance', loader: 'fabric', loader_version: '0.16', minecraft_version: '1.21', is_locked: false, last_launched_at: null },
    ];

    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        // --- App-level commands ---
        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          // Return sensible defaults for settings pages may query.
          return Promise.resolve(null);
        }
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command.startsWith('plugin:event|') || command.startsWith('plugin:shell|')) return Promise.resolve(1);

        // --- Registry (needed by sidebar and browse) ---
        if (command === 'get_registry_status') {
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

        // --- ModDetail commands ---
        if (command === 'get_registry_item') {
          // Return null to simulate "not in registry"
          return Promise.resolve(null);
        }
        if (command === 'fetch_modrinth_project') {
          // Return null to simulate "not a Modrinth project"
          return Promise.resolve(null);
        }
        if (command === 'get_auth_status') return Promise.resolve(false);
        if (command === 'get_github_profile') return Promise.resolve(null);
        if (command === 'get_flag_rate_limit') return Promise.resolve({ remaining: 10, reset_at: null });
        if (command === 'list_mod_reviews') return Promise.resolve([]);
        // Multi-session launch state is a list; the backend never returns null.
        if (command === 'query_launch_state') return Promise.resolve([]);
        if (command === 'list_instances') return Promise.resolve(instances);
        if (command === 'is_modrinth_enabled') return Promise.resolve(false);
        if (command === 'list_manifest_loaders') return Promise.resolve(['fabric', 'forge']);
        if (command === 'list_manifest_mc_versions') return Promise.resolve(['1.21', '1.20.4']);
        if (command === 'get_curated_annotation') return Promise.resolve(null);
        if (command === 'list_raw_modrinth_versions') return Promise.resolve([]);
        if (command === 'list_mod_versions') return Promise.resolve({ items: [], hasMore: false });
        if (command === 'list_mod_versions_load_more') return Promise.resolve({ items: [], hasMore: false });

        // --- InstanceEditor commands ---
        if (command === 'get_instance_detail') {
          // For invalid-instance variant, return null.
          // For invalid-mod variant, return a valid instance (needed by mod detail
          // to render the install-flow instance picker).
          if (options.variant === 'invalid-instance') return Promise.resolve(null);
          return Promise.resolve({
            row: instances[0],
            manifest: {
              instance_id: instances[0].instance_id,
              name: instances[0].name,
              minecraft_version: instances[0].minecraft_version,
              loader: instances[0].loader,
              loader_version: instances[0].loader_version,
              is_locked: false,
              mods: [], resourcepacks: [], shaders: [], datapacks: [], worlds: [],
              user_preferences: {},
            },
          });
        }
        if (command === 'list_categories') return Promise.resolve([]);
        if (command === 'list_snapshots') return Promise.resolve([]);
        if (command === 'list_loadout_profiles') return Promise.resolve([]);
        if (command === 'list_loader_versions') return Promise.resolve([]);
        if (command === 'browse_items') return Promise.resolve([]);
        if (command === 'list_pack_mods') return Promise.resolve([]);

        // Catch-all for any unhandled list/inspect command.
        if (command.startsWith('list_') || command.startsWith('get_')) {
          return Promise.resolve(null);
        }

        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  }, { options });
}

/**
 * Navigate to a destination by pushing history state and dispatching a
 * popstate event. This mirrors how the app processes external navigation.
 */
async function navigateToDestination(page: Page, dest: Record<string, unknown>) {
  await page.evaluate((dest) => {
    window.history.pushState({ __agora: dest }, '');
    window.dispatchEvent(new PopStateEvent('popstate', { state: { __agora: dest } }));
  }, dest);
}

test('invalid mod ID shows "Mod not found" with Back button that returns to Browse', async ({ page }) => {
  await installMock(page, { variant: 'invalid-mod' });
  await page.goto('/');

  // Navigate to Browse first so Back has somewhere to return.
  await page.getByRole('button', { name: 'Browse', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Browse', level: 2 })).toBeVisible();

  // Programmatically navigate to a non-existent mod detail.
  await navigateToDestination(page, { type: 'mod-detail', itemId: 'non-existent-mod' });

  // The page should show the deliberate "Mod not found." error.
  await expect(page.getByText('Mod not found.')).toBeVisible();

  // A Back button is present for recovery.
  const backButton = page.getByRole('button', { name: '← Back' });
  await expect(backButton).toBeVisible();

  // Clicking Back returns to Browse.
  await backButton.click();
  await expect(page.getByRole('heading', { name: 'Browse', level: 2 })).toBeVisible();
});

test('invalid instance ID shows "Instance not found" with Back button that returns to Instances', async ({ page }) => {
  await installMock(page, { variant: 'invalid-instance' });
  await page.goto('/');

  // Navigate to Instances first so Back has somewhere to return.
  await page.getByRole('button', { name: 'My Instances' }).click();
  await expect(page.getByRole('heading', { name: 'My Instances', level: 2 })).toBeVisible();

  // Programmatically navigate to a non-existent instance detail.
  await navigateToDestination(page, { type: 'instance-detail', instanceId: 'non-existent-instance' });

  // The page should show the deliberate "Instance not found." error.
  await expect(page.getByText('Instance not found.')).toBeVisible();

  // A Back button is present for recovery.
  const backButton = page.getByRole('button', { name: '← Back' });
  await expect(backButton).toBeVisible();

  // Clicking Back returns to My Instances.
  await backButton.click();
  await expect(page.getByRole('heading', { name: 'My Instances', level: 2 })).toBeVisible();
});


