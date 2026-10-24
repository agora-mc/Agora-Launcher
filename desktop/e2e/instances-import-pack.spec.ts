import { test, expect, type Page } from '@playwright/test';

// Top-level "Import pack file" on My Instances (creates a NEW instance) and
// the creation date on instance cards.

async function installMock(
  page: Page,
  options: {
    instances: Record<string, unknown>[];
    pickedPath: string | null;
    report?: unknown;
    /** Fields merged over the review plan the mock returns. */
    plan?: Record<string, unknown>;
    /** Fields merged over the instance the import creates. */
    importedExtra?: Record<string, unknown>;
  },
) {
  await page.addInitScript(({ options }) => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const calls: { command: string; args: Record<string, unknown> }[] = [];
    let imported = false;
    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        calls.push({ command, args });
        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'launch_mode') return Promise.resolve('direct');
          if (key === 'last_home_visit') return Promise.resolve(null);
          return Promise.resolve(false);
        }
        if (command === 'get_registry_status') {
          return Promise.resolve({
            has_cached_db: true, cached_tag: 'test', cached_schema_version: 5,
            latest_tag: 'test', update_available: false, checked: true, message: 'Catalog ready.',
          });
        }
        if (command === 'list_categories' || command === 'for_you_items') return Promise.resolve([]);
        if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
        if (command === 'list_instances') {
          return Promise.resolve(imported
            ? [...options.instances, {
              instance_id: 'imported-pack', name: 'Imported Pack', minecraft_version: '1.21',
              loader: 'fabric', loader_version: '0.16.9', is_locked: false, last_launched_at: null,
              created_at: '2026-10-21T10:00:00Z',
              ...(options.importedExtra ?? {}),
            }]
            : options.instances);
        }
        if (command === 'pick_open_file') return Promise.resolve(options.pickedPath);
        if (command === 'plan_pack_import') {
          return Promise.resolve({
            pack_name: 'Imported Pack',
            instance: {
              id: 'imported-pack', name: 'Imported Pack', minecraft_version: '1.21',
              loader: 'fabric', loader_version: '0.16.9',
            },
            entries: [
              { name: 'sodium.jar', display_name: 'Sodium', source: 'curated', checkable: true, reason: null, action: null },
              { name: 'xaero.jar', display_name: null, source: 'modrinth', checkable: true, reason: 'Needs Modrinth, which is off.', action: 'enable_modrinth' },
            ],
            file_sha256: 'c'.repeat(64),
            file_size: 512,
            name_taken: false,
            suggested_name: 'Imported Pack',
            left_out: [],
            problems: [],
            notes: [],
            ...(options.plan ?? {}),
          });
        }
        if (command === 'import_instance_pack') {
          imported = true;
          return Promise.resolve({ instance_id: 'imported-pack', report: options.report ?? null });
        }
        if (command === 'set_content_provider_enabled') return Promise.resolve([]);
        if (command === 'retry_pack_import_entries') {
          const entries = args.entries as { filename: string }[];
          return Promise.resolve({
            instance_id: 'imported-pack', instance_name: 'Imported Pack', total: entries.length,
            installed: entries.length, warnings: [],
            outcomes: entries.map((entry) => ({
              name: entry.filename, status: 'installed', verified_against_pack: true,
              reason: null, action: null, entry,
            })),
          });
        }
        if (command === 'plugin:event|listen') return Promise.resolve(1);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command === 'query_launch_state' || command === 'check_instance_updates') return Promise.resolve([]);
        if (command === 'list_instance_templates') return Promise.resolve([]);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __calls: calls,
    });
  }, { options });
}

const calls = (page: Page, command: string) =>
  page.evaluate(
    (name) => ((window as any).__calls as { command: string; args: unknown }[]).filter((c) => c.command === name),
    command,
  );

test.describe('My Instances: import a pack file', () => {
  test('empty state offers Import pack file, which creates a new instance', async ({ page }) => {
    await installMock(page, { instances: [], pickedPath: 'C:\\packs\\my.agora-pack.json' });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await expect(page.getByText('No instances yet.')).toBeVisible();

    // Two entry points: the header and the empty state.
    await expect(page.getByRole('button', { name: 'Import pack file' })).toHaveCount(2);
    await page.getByRole('button', { name: 'Import pack file' }).last().click();

    // Nothing is imported until the review's Import is pressed.
    await expect(page.getByTestId('pack-import-review')).toContainText('Import Imported Pack?');
    expect(await calls(page, 'import_instance_pack')).toHaveLength(0);
    await page.getByRole('button', { name: 'Import', exact: true }).click();

    await expect(page.getByRole('heading', { name: 'Imported Pack' })).toBeVisible();
    // The "importing" banner goes away once the import has finished.
    await expect(page.getByRole('status').filter({ hasText: 'new instance' })).toHaveCount(0);
    const imports = await calls(page, 'import_instance_pack');
    expect(imports).toHaveLength(1);
    expect((imports[0].args as Record<string, unknown>).sourcePath).toBe('C:\\packs\\my.agora-pack.json');
    // The import is held to the bytes the review recorded.
    expect((imports[0].args as Record<string, unknown>).reviewedSha256).toBe('c'.repeat(64));
  });

  test('a partial import says which mod is missing and retries it after Modrinth is turned on', async ({ page }) => {
    const entry = (filename: string, registry: string | null) => ({
      filename, version: '1', registry_id: registry, modrinth_id: registry ? null : 'xaero',
      sha256: 'a'.repeat(64), source: registry ? 'registry' : 'modrinth_raw',
    });
    const done = (filename: string) => ({
      name: filename, status: 'installed', verified_against_pack: true, reason: null, action: null,
      entry: entry(filename, 'sodium'),
    });
    await installMock(page, {
      instances: [],
      pickedPath: 'C:\\packs\\friend.agora-pack.json',
      report: {
        instance_id: 'imported-pack', instance_name: 'Imported Pack', total: 5, installed: 4, warnings: [],
        outcomes: [
          {
            name: 'xaero.jar', status: 'needs_action', verified_against_pack: false, action: 'enable_modrinth',
            reason: 'Needs Modrinth, which is off. Turning it on sends requests for this mod to Modrinth.',
            entry: entry('xaero.jar', null),
          },
          done('a.jar'), done('b.jar'), done('c.jar'), done('d.jar'),
        ],
      },
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Import pack file' }).last().click();
    await page.getByRole('button', { name: 'Import', exact: true }).click();

    const summary = page.getByTestId('pack-import-summary');
    await expect(summary.getByText('4 of 5 mods installed.')).toBeVisible();
    await expect(summary.getByText(/xaero\.jar/)).toBeVisible();
    await expect(summary.getByText(/Needs Modrinth, which is off/)).toBeVisible();

    // Allowing Modrinth here is for this import only: the setting is not touched.
    await summary.getByRole('button', { name: 'Allow Modrinth for this import' }).click();
    await expect(summary.getByText('5 of 5 mods installed.')).toBeVisible();
    const retries = await calls(page, 'retry_pack_import_entries');
    expect(retries).toHaveLength(1);
    const sent = retries[0].args as { entries: { filename: string }[]; allowModrinthOnce: boolean };
    expect(sent.entries.map((e) => e.filename)).toEqual(['xaero.jar']);
    expect(sent.allowModrinthOnce).toBe(true);
    expect(await calls(page, 'set_content_provider_enabled')).toHaveLength(0);
  });

  test('the summary sits in the page flow, so it never covers the sidebar, and can be dismissed', async ({ page }) => {
    await installMock(page, {
      instances: [],
      pickedPath: 'C:\\packs\\friend.agora-pack.json',
      report: {
        instance_id: 'imported-pack', instance_name: 'Imported Pack', total: 2, installed: 1, warnings: [],
        outcomes: [
          {
            name: 'xaero.jar', status: 'needs_action', verified_against_pack: false, action: 'enable_modrinth',
            reason: 'Needs Modrinth, which is off.', entry: { filename: 'xaero.jar', version: '1', registry_id: null, modrinth_id: 'xaero', sha256: null, source: 'modrinth_raw' },
          },
          {
            name: 'a.jar', status: 'installed', verified_against_pack: true, reason: null, action: null,
            entry: { filename: 'a.jar', version: '1', registry_id: 'sodium', modrinth_id: null, sha256: null, source: 'registry' },
          },
        ],
      },
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Import pack file' }).last().click();
    await page.getByRole('button', { name: 'Import', exact: true }).click();

    const summary = page.getByTestId('pack-import-summary');
    await expect(summary).toBeVisible();
    const box = await summary.boundingBox();
    const navigation = await page.getByRole('navigation', { name: 'Main navigation' }).boundingBox();
    expect(box).not.toBeNull();
    expect(navigation).not.toBeNull();
    // The card does not overlap the main navigation.
    const overlaps = box!.x < navigation!.x + navigation!.width && box!.x + box!.width > navigation!.x;
    expect(overlaps).toBe(false);

    await page.getByRole('button', { name: 'Dismiss import results' }).click();
    await expect(summary).toHaveCount(0);
  });

  test('a .mrpack is reviewed with a free name, and the new instance says which file it came from', async ({ page }) => {
    await installMock(page, {
      instances: [],
      pickedPath: 'C:\\packs\\spiff-fabric-1.20.4.mrpack',
      plan: {
        name_taken: true,
        suggested_name: 'Imported Pack (2)',
        entries: [
          { name: 'mods/sodium.jar', display_name: 'sodium.jar', source: 'modrinth', checkable: true, reason: null, action: null },
          { name: 'shaderpacks/BSL.zip', display_name: 'BSL.zip', source: 'bundled', checkable: false, reason: null, action: null },
        ],
        left_out: ['saves/world/level.dat (outside the folders Agora copies from a pack)'],
      },
      importedExtra: { name: 'Imported Pack (2)', import_source: 'mrpack:spiff-fabric-1.20.4.mrpack' },
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Import pack file' }).last().click();

    const review = page.getByTestId('pack-import-review');
    await expect(review.getByLabel('Instance name')).toHaveValue('Imported Pack (2)');
    await expect(review).toContainText('1 from Modrinth, 1 bundled in the pack');
    await expect(review).toContainText('saves/world/level.dat');
    expect(await calls(page, 'import_instance_pack')).toHaveLength(0);
    await review.getByRole('button', { name: 'Import as a copy' }).click();

    await expect(page.getByRole('heading', { name: 'Imported Pack (2)' })).toBeVisible();
    await expect(page.getByText(/^Imported .* from spiff-fabric-1\.20\.4\.mrpack$/)).toBeVisible();
    const imports = await calls(page, 'import_instance_pack');
    expect((imports[0].args as Record<string, unknown>).name).toBe('Imported Pack (2)');
  });

  test('header button works with existing instances and cancelling imports nothing', async ({ page }) => {
    await installMock(page, {
      instances: [{
        instance_id: 'a', name: 'Alpha', minecraft_version: '1.21', loader: 'fabric',
        loader_version: '0.16.9', is_locked: false, last_launched_at: null,
        created_at: '2026-03-05T12:00:00Z',
      }],
      pickedPath: null,
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Import pack file' }).click();
    await expect.poll(async () => (await calls(page, 'pick_open_file')).length).toBe(1);
    expect(await calls(page, 'import_instance_pack')).toHaveLength(0);
    await expect(page.getByRole('status')).toHaveCount(0);
  });

  test('instance cards show a creation date, and omit it when unreadable', async ({ page }) => {
    await installMock(page, {
      instances: [
        {
          instance_id: 'a', name: 'Alpha', minecraft_version: '1.21', loader: 'fabric',
          loader_version: '0.16.9', is_locked: false, last_launched_at: null,
          created_at: '2026-03-05T12:00:00Z',
        },
        {
          instance_id: 'b', name: 'Beta', minecraft_version: '1.21', loader: 'fabric',
          loader_version: '0.16.9', is_locked: false, last_launched_at: null,
          created_at: 'not a date',
        },
      ],
      pickedPath: null,
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await expect(page.getByText(/^Created Mar 5, 2026$/)).toHaveCount(1);
    await expect(page.getByText(/^Created /)).toHaveCount(1);
  });
});
