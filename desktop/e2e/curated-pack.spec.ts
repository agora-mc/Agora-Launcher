import { test, expect, type Page } from '@playwright/test';

/**
 * Curated pack install dialog: locked releases, the flexible recipe, and the
 * rule that a missing *required* mod blocks the install while recommended and
 * optional ones are only left out. Planning is core's job; these tests mock
 * `plan_curated_pack` and check what the dialog does with the answer.
 */
async function installMock(page: Page, options: { releases: boolean }) {
  await page.addInitScript(({ options }) => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
    (window as unknown as Record<string, unknown>).__calls = calls;

    const pack = {
      id: 'optimized-survival',
      name: 'Community Optimized Survival',
      content_type: 'pack',
      download_strategy: 'curated_pack',
      source_identifier: 'optimized-survival',
      sha256: 'a'.repeat(64),
      upvotes: 0, downvotes: 0, net_score: 0, velocity: 0,
      status: 'active',
      is_immune: false, immunity_reason: null, allow_comments: true,
      icon_url: null, gallery_urls_json: null, date_added: null,
      compatible_versions_json: JSON.stringify([{ mc_version: '1.21', loader: 'fabric', mod_version: '1.0.0' }]),
      description: 'A test pack.', body_markdown: null, page_url: null, license_id: null,
      source_updated_at: null, modrinth_id: null,
    };

    const plannedMod = (modId: string, status: string) => ({
      modId, status, sourceType: 'curated', itemId: modId,
      version: '1.0.0', displayVersion: '1.0.0', pinned: true,
    });

    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        calls.push({ command, args });
        if (command === 'get_setting') return Promise.resolve(args.key === 'onboarding_complete' ? true : null);
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command.startsWith('plugin:')) return Promise.resolve(1);
        if (command === 'get_registry_status') {
          return Promise.resolve({
            has_cached_db: true, cached_tag: 'test', cached_schema_version: 9, latest_tag: 'test',
            update_available: false, checked: true, message: 'Catalog ready.',
          });
        }
        if (command === 'get_registry_item') return Promise.resolve(args.itemId === pack.id || args.id === pack.id ? pack : pack);
        if (command === 'fetch_modrinth_project') return Promise.resolve(null);
        if (command === 'get_auth_status') return Promise.resolve(false);
        if (command === 'is_modrinth_enabled') return Promise.resolve(false);
        if (command === 'query_launch_state') return Promise.resolve([]);
        if (command === 'list_instances') return Promise.resolve([]);
        if (command === 'list_manifest_loaders') return Promise.resolve(['fabric', 'forge']);
        if (command === 'list_manifest_mc_versions') return Promise.resolve(['1.21', '1.20.4']);
        if (command === 'list_loader_versions') {
          return Promise.resolve([{ loader: 'fabric', mc_version: String(args.mcVersion ?? '1.21'), loader_version: '0.19.5', file_type: 'profile_json' }]);
        }
        if (command === 'list_pack_versions') {
          return Promise.resolve(options.releases
            ? [{ pack_id: pack.id, version: '1.0.0', minecraft_version: '1.21', loader: 'fabric', loader_version: '0.19.5', changelog: 'First release.' }]
            : []);
        }
        if (command === 'plan_curated_pack') {
          const selection = args.selection as { mode: string; minecraftVersion?: string };
          if (selection.mode === 'locked') {
            return Promise.resolve({
              packId: pack.id, packVersion: '1.0.0',
              target: { minecraftVersion: '1.21', loader: 'fabric', loaderVersion: '0.19.5' },
              mods: [plannedMod('sodium', 'required'), plannedMod('iris', 'recommended')],
              dropped: [], blocking: [],
            });
          }
          // Flexible on an old version: the core mod has no build.
          return Promise.resolve({
            packId: pack.id, packVersion: null,
            target: { minecraftVersion: selection.minecraftVersion, loader: 'fabric', loaderVersion: null },
            mods: [],
            dropped: [{ modId: 'iris', status: 'recommended', reason: 'No build for Minecraft 1.20.4 with fabric.' }],
            blocking: [{ modId: 'sodium', status: 'required', reason: 'No build for Minecraft 1.20.4 with fabric.' }],
          });
        }
        if (command === 'create_instance') {
          const request = args.request as Record<string, unknown>;
          return Promise.resolve({
            instance_id: request.instance_id, name: request.name, loader: request.loader,
            loader_version: request.loader_version, minecraft_version: request.minecraft_version,
            is_locked: false, last_launched_at: null,
          });
        }
        // The install review is out of scope here: leave it pending.
        if (command === 'resolve_install_plan') return new Promise(() => {});
        if (command.startsWith('list_') || command.startsWith('get_')) return Promise.resolve(null);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  }, { options });
}

async function openPackDialog(page: Page) {
  await page.goto('/');
  await page.evaluate(() => {
    const dest = { type: 'mod-detail', itemId: 'optimized-survival' };
    window.history.pushState({ __agora: dest }, '');
    window.dispatchEvent(new PopStateEvent('popstate', { state: { __agora: dest } }));
  });
  await page.getByRole('button', { name: 'Create Instance from Pack' }).click();
  await expect(dialog(page)).toBeVisible();
}

function dialog(page: Page) {
  return page.getByRole('dialog', { name: /Create Instance from Pack/ });
}

async function calls(page: Page) {
  return page.evaluate(() => (window as unknown as { __calls: Array<{ command: string; args: Record<string, unknown> }> }).__calls);
}

test('a pack with releases defaults to its newest locked release', async ({ page }) => {
  await installMock(page, { releases: true });
  await openPackDialog(page);

  await expect(page.getByRole('radio', { name: /Pack release/ })).toBeChecked();
  await expect(page.getByText('Minecraft 1.21 · fabric 0.19.5')).toBeVisible();
  // Create is not offered until the pack has been checked.
  await expect(page.getByRole('button', { name: 'Create', exact: true })).toHaveCount(0);

  await page.getByRole('button', { name: 'Check pack' }).click();
  await expect(page.getByTestId('curated-pack-plan')).toContainText('2 mods will be installed on Minecraft 1.21');
  await expect(page.getByRole('button', { name: 'Create', exact: true })).toBeEnabled();

  const plan = (await calls(page)).find((call) => call.command === 'plan_curated_pack');
  expect(plan?.args.selection).toEqual({ mode: 'locked', packVersion: '1.0.0' });
});

test('flexible mode blocks when a required mod has no build and lists what was left out', async ({ page }) => {
  await installMock(page, { releases: true });
  await openPackDialog(page);

  await page.getByRole('radio', { name: /Flexible/ }).check();
  await dialog(page).getByLabel('Minecraft version', { exact: true }).selectOption('1.20.4');
  await page.getByRole('button', { name: 'Check pack' }).click();

  const summary = page.getByTestId('curated-pack-plan');
  await expect(summary).toContainText("Can't install: a required mod is missing.");
  await expect(summary).toContainText('Nothing will be installed on Minecraft 1.20.4');
  await expect(summary).toContainText('sodium');
  await expect(summary).toContainText('Left out (1)');
  await expect(page.getByRole('button', { name: 'Create', exact: true })).toBeDisabled();
});

test('changing the target discards a stale plan', async ({ page }) => {
  await installMock(page, { releases: true });
  await openPackDialog(page);

  await page.getByRole('button', { name: 'Check pack' }).click();
  await expect(page.getByTestId('curated-pack-plan')).toBeVisible();
  await page.getByRole('radio', { name: /Flexible/ }).check();
  await expect(page.getByTestId('curated-pack-plan')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Check pack' })).toBeVisible();
});

test('a pack without releases plans its flexible recipe for the chosen target', async ({ page }) => {
  await installMock(page, { releases: false });
  await openPackDialog(page);

  await expect(page.getByRole('radio', { name: /Pack release/ })).toHaveCount(0);
  await expect(dialog(page).getByLabel('Minecraft version', { exact: true })).toHaveValue('1.21');
  await page.getByRole('button', { name: 'Check pack' }).click();
  await expect(page.getByTestId('curated-pack-plan')).toBeVisible();
  const plan = (await calls(page)).find((call) => call.command === 'plan_curated_pack');
  expect(plan?.args.selection).toEqual({ mode: 'flexible', minecraftVersion: '1.21', loader: 'fabric' });
});

test('Create builds the instance from the plan and hands only planned mods to the review', async ({ page }) => {
  await installMock(page, { releases: true });
  await openPackDialog(page);

  await page.getByRole('button', { name: 'Check pack' }).click();
  await page.getByRole('button', { name: 'Create', exact: true }).click();

  await expect.poll(async () => (await calls(page)).some((call) => call.command === 'create_instance')).toBe(true);
  const create = (await calls(page)).find((call) => call.command === 'create_instance');
  expect(create?.args.request).toMatchObject({
    minecraft_version: '1.21',
    loader: 'fabric',
    loader_version: '0.19.5',
    is_modpack: true,
  });
});
