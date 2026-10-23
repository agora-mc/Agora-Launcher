import { test, expect, type Page } from '@playwright/test';

// Top-level "Import pack file" on My Instances (creates a NEW instance) and
// the creation date on instance cards.

async function installMock(
  page: Page,
  options: { instances: Record<string, unknown>[]; pickedPath: string | null; report?: unknown },
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
            }]
            : options.instances);
        }
        if (command === 'pick_open_file') return Promise.resolve(options.pickedPath);
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

    await expect(page.getByRole('heading', { name: 'Imported Pack' })).toBeVisible();
    // The "importing" banner goes away once the import has finished.
    await expect(page.getByRole('status').filter({ hasText: 'new instance' })).toHaveCount(0);
    const imports = await calls(page, 'import_instance_pack');
    expect(imports).toHaveLength(1);
    expect((imports[0].args as Record<string, unknown>).sourcePath).toBe('C:\\packs\\my.agora-pack.json');
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

    const summary = page.getByTestId('pack-import-summary');
    await expect(summary.getByText('4 of 5 mods installed.')).toBeVisible();
    await expect(summary.getByText(/xaero\.jar/)).toBeVisible();
    await expect(summary.getByText(/Needs Modrinth, which is off/)).toBeVisible();

    await summary.getByRole('button', { name: 'Turn on Modrinth for this' }).click();
    await expect(summary.getByText('5 of 5 mods installed.')).toBeVisible();
    const retries = await calls(page, 'retry_pack_import_entries');
    expect(retries).toHaveLength(1);
    const sent = (retries[0].args as { entries: { filename: string }[] }).entries;
    expect(sent.map((e) => e.filename)).toEqual(['xaero.jar']);
    expect((await calls(page, 'set_content_provider_enabled'))[0].args).toEqual({ providerId: 'modrinth', enabled: true });
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
