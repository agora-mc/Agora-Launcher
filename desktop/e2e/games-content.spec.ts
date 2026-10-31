import { test, expect, type Page } from '@playwright/test';
import { expectNoUnmockedCommands, installGamesMock } from './support/games-mock';

/**
 * The content slice (MASTER_SPEC §26.6, §26.14): the Library's store, an instance's Content tab,
 * and the FOMOD wizard. Replies come from the shared table in `support/games-mock.ts`. Two things
 * this spec adds on top of the table, both in the page before the app starts:
 *  - `failures`: a command that rejects with a backend error (the table only resolves).
 *  - `fomodInstall`: `games_fomod_install` answers differently for a preview and an install.
 * Anything else the app calls and the table does not answer fails the test.
 */

const skyrim = {
  gameId: 'skyrim-se',
  name: 'Skyrim Special Edition',
  installs: [
    {
      installId: 'steam-489830',
      store: 'steam',
      name: 'Skyrim Special Edition',
      location: 'D:/Games/Skyrim Special Edition',
      storeVersion: '1.6.1170',
      storeBuild: null,
      addOnCount: 0,
      installBytes: null,
      runtime: {
        status: 'identified',
        runtime: { game: 'skyrim-se', store: 'steam', version: '1.6.1170', build: null },
        source: 'executable',
      },
    },
  ],
};

/** The boot commands the shell answers with empty data (the same set the shell spec uses). */
const BOOT_REPLIES = {
  get_windows_accent_color: null,
  get_registry_status: { has_cached_db: false, checked: true, message: 'Missing' },
  check_registry_update: { has_cached_db: false, checked: true, message: 'Missing' },
  list_instances: [],
  list_snapshots: [],
  for_you_items: [],
  query_launch_state: [],
  check_all_instance_health: [],
  take_pending_cli_launch: null,
  get_governance_config: null,
  plugins_enabled: false,
  list_plugins: [],
  list_plugin_contributions: [],
  get_connectivity_status: { state: 'unknown', last_outcome_at: null },
  list_recent_play: [],
  // The Library tab also carries the catalog and instance lists of the other slices.
  games_instance_list: { instances: [] },
  games_catalog_list: { entries: [], instances: [] },
};

const odin = {
  itemId: '0151b2a02801347e563b9265a94af8cbe66b13bce1001a6a8d9e6556a075bd0e',
  name: 'Odin Patch',
  fileCount: 1,
  totalSize: 6772,
  sourceKind: 'archive',
  sourceLabel: 'Archive: odin.zip',
  addedAtUnixMs: 1,
  gameIds: [],
};

const sky = {
  itemId: 'cccc3333dddd4444eeee5555ffff6666',
  name: 'Sky Mod',
  fileCount: 4,
  totalSize: 2048,
  sourceKind: 'archive',
  sourceLabel: 'Archive: sky.7z',
  addedAtUnixMs: 1,
  gameIds: [],
};

const oneRow = {
  position: 1,
  itemId: odin.itemId,
  layerId: `content:${odin.itemId.slice(0, 16)}`,
  name: 'Odin Patch',
  sourceKind: 'archive',
  sourceLabel: 'Archive: odin.zip',
  enabled: true,
  mountPath: 'Data',
  sourcePath: '',
};

interface Overrides {
  failures?: Record<string, { code: string; message: string }>;
  fomodInstall?: { preview: unknown; installed: unknown };
}

async function boot(page: Page, replies: Record<string, unknown>, overrides: Overrides = {}) {
  await installGamesMock(page, {
    replies: { ...BOOT_REPLIES, games_list: { games: [skyrim] }, ...replies },
    settings: { active_game: 'skyrim-se' },
  });
  await page.addInitScript(
    ({ failures, fomodInstall }) => {
      const target = window as unknown as {
        __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
      };
      const inner = target.__TAURI_INTERNALS__.invoke;
      target.__TAURI_INTERNALS__.invoke = (command: string, args: Record<string, unknown> = {}) => {
        if (Object.prototype.hasOwnProperty.call(failures, command)) {
          return Promise.reject(failures[command]);
        }
        if (fomodInstall && command === 'games_fomod_install') {
          return Promise.resolve(args.preview === true ? fomodInstall.preview : fomodInstall.installed);
        }
        return inner(command, args);
      };
    },
    { failures: overrides.failures ?? {}, fomodInstall: overrides.fomodInstall ?? null },
  );
}

/** Open an instance's Content tab directly: the app restores a destination from history state. */
async function openInstanceContent(page: Page) {
  await page.addInitScript(() => {
    if (!window.history.state?.__agora) {
      window.history.replaceState(
        { __agora: { type: 'game-instance', gameId: 'skyrim-se', instanceId: 'inst-1', page: 'content' }, __agoraDepth: 0 },
        '',
      );
    }
  });
}

function sidebarNav(page: Page) {
  return page.getByRole('navigation', { name: 'Main navigation' });
}

async function openLibrary(page: Page) {
  await page.goto('/');
  await sidebarNav(page).getByRole('button', { name: 'Library', exact: true }).click();
}

test.afterEach(async ({ page }) => {
  await expectNoUnmockedCommands(page);
});

test('an empty library says what to do, and adding an archive confirms it', async ({ page }) => {
  await boot(page, {
    games_content_list: { items: [], damaged: [] },
    pick_open_file: '/mods/Odin Patch.zip',
    games_content_add: { status: 'added', item: odin, objectsNew: 1, objectsRestored: 0 },
  });
  await openLibrary(page);

  await expect(page.getByText('Your library is empty.')).toBeVisible();
  await page.getByRole('button', { name: 'Add an archive…' }).click();
  await expect(page.getByText('Added Odin Patch (1 file).', { exact: true })).toBeVisible();
});

test('a stored item can be checked, and the check says what is wrong with it', async ({ page }) => {
  await boot(page, {
    games_content_list: { items: [odin], damaged: [] },
    games_content_verify: {
      itemId: odin.itemId,
      checked: 1,
      hashed: 1,
      problems: [
        { path: 'Odin.esp', kind: 'hash_mismatch', message: 'The file changed: its contents no longer match the recorded hash.' },
      ],
    },
  });
  await openLibrary(page);

  const item = page.getByTestId('library-item');
  await expect(item.getByText('Archive: odin.zip')).toBeVisible();
  await item.getByRole('button', { name: 'Check everything' }).click();
  await expect(item.getByText('1 problem found:')).toBeVisible();
  await expect(item).toContainText('The file changed: its contents no longer match the recorded hash.');
});

test('removing a stored item asks first, then removes it', async ({ page }) => {
  await boot(page, {
    games_content_list: { items: [odin], damaged: [] },
    games_content_remove: null,
  });
  await openLibrary(page);

  await page.getByTestId('library-item').getByRole('button', { name: 'Remove' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Remove Odin Patch from your library?');
  await dialog.getByRole('button', { name: 'Remove' }).click();
  await expect(page.getByText('Removed Odin Patch.', { exact: true })).toBeVisible();
});

test('a library that cannot be read says so and offers a retry, rather than looking empty', async ({ page }) => {
  await boot(page, { games_content_list: { items: [], damaged: [] } }, {
    failures: { games_content_list: { code: 'core_failed', message: 'The store index is damaged.' } },
  });
  await openLibrary(page);

  const alert = page.getByRole('alert');
  await expect(alert).toContainText('Your library could not be read: The store index is damaged.');
  await expect(alert.getByRole('button', { name: 'Try again' })).toBeVisible();
  await expect(page.getByText('Your library is empty.')).toHaveCount(0);
});

test('an instance’s content lists its load order and says where each item goes', async ({ page }) => {
  await boot(page, {
    games_instance_content_list: { instanceId: 'inst-1', items: [oneRow] },
    games_content_list: { items: [], damaged: [] },
  });
  await openInstanceContent(page);
  await page.goto('/');

  const row = page.getByTestId('instance-content-row');
  await expect(row).toContainText('Odin Patch');
  await expect(row).toContainText('On');
  await expect(row).toContainText('Installs into the Data folder');
  await expect(row.getByRole('button', { name: 'Turn off' })).toBeVisible();
});

test('an instance whose content cannot be read says so, and does not show an empty tab', async ({ page }) => {
  await boot(page, { games_content_list: { items: [], damaged: [] } }, {
    failures: { games_instance_content_list: { code: 'core_failed', message: 'The instance manifest is damaged.' } },
  });
  await openInstanceContent(page);
  await page.goto('/');

  await expect(page.getByRole('alert').first()).toContainText('The instance manifest is damaged.');
  await expect(page.getByText('This instance has no content yet.')).toHaveCount(0);
});

test('an archive with an installer asks its questions, previews the files, then installs', async ({ page }) => {
  const choices = [{ step: 'Look', group: 'Body', plugins: ['Slim'] }];
  await boot(
    page,
    {
      games_instance_content_list: { instanceId: 'inst-1', items: [] },
      games_content_list: { items: [sky], damaged: [] },
      games_instance_content_add: {
        status: 'needs_installer',
        itemId: sky.itemId,
        reason: 'it has a FOMOD installer',
        topLevel: ['fomod', 'Sky.esp'],
      },
      games_fomod_show: {
        installer: {
          itemId: sky.itemId,
          title: 'Sky Installer',
          author: 'Pat',
          version: '1.0',
          description: null,
          requires: null,
          alwaysInstalledCount: 0,
          conditionalInstallCount: 0,
          steps: [
            {
              name: 'Look',
              visible: null,
              groups: [
                {
                  name: 'Body',
                  rule: 'select exactly one',
                  singleChoice: true,
                  plugins: [
                    { name: 'Slim', description: 'Lighter textures', pluginType: 'recommended' },
                    { name: 'Full', description: 'Everything', pluginType: 'optional' },
                  ],
                },
              ],
            },
          ],
        },
        suggestedChoices: choices,
      },
    },
    {
      fomodInstall: {
        preview: {
          status: 'preview',
          files: [{ destination: 'Sky.esp', source: 'Sky.esp', size: 2048 }],
          totalSize: 2048,
          notes: [],
          choices,
          dataFolder: 'Data',
        },
        installed: {
          status: 'installed',
          itemId: 'dddd4444',
          name: 'Sky Installer (FOMOD)',
          fileCount: 1,
          totalSize: 2048,
          notes: [],
          addedToInstance: true,
          layerId: 'content:dddd4444',
          mountPath: 'Data',
        },
      },
    },
  );
  await openInstanceContent(page);
  await page.goto('/');

  await expect(page.getByText('This instance has no content yet.')).toBeVisible();
  await page.getByRole('combobox').selectOption({ label: 'Sky Mod' });
  await page.getByRole('button', { name: 'Add to instance' }).click();

  const wizard = page.getByRole('dialog');
  await expect(wizard).toContainText('Sky Installer');
  await expect(wizard.getByRole('radio', { name: /Slim/ })).toBeChecked();
  await wizard.getByRole('button', { name: 'Review files' }).click();

  await expect(wizard).toContainText('1 file (2.0 KB), placed in Data:');
  await expect(wizard.getByRole('list', { name: 'Files to install' })).toContainText('Sky.esp');
  await wizard.getByRole('button', { name: 'Install these files' }).click();
  await expect(wizard).toContainText('Installed Sky Installer (FOMOD): 1 file.');
  await expect(wizard).toContainText('It has been added to this instance.');
});

test('the Library shows the mods for this game and those that name none; a switch shows the rest', async ({ page }) => {
  const forFallout = { ...sky, itemId: 'ffff0001', name: 'Fallout Texture Pack', gameIds: ['fallout-4'] };
  const forSkyrim = { ...sky, itemId: 'ffff0002', name: 'Skyrim Lighting', gameIds: ['skyrim-se'] };
  await boot(page, { games_content_list: { items: [odin, forFallout, forSkyrim], damaged: [] } });
  await openLibrary(page);

  const rows = page.getByTestId('library-item');
  await expect(rows).toHaveCount(2);
  await expect(page.getByText('Odin Patch')).toBeVisible();
  await expect(page.getByText('Skyrim Lighting')).toBeVisible();
  await expect(page.getByText('Fallout Texture Pack')).toHaveCount(0);

  await page.getByLabel('Show mods for every game').check();
  await expect(rows).toHaveCount(3);
  await expect(page.getByText('Fallout Texture Pack')).toBeVisible();
});

test('a stored record the store cannot read is listed with its reason and can be removed', async ({ page }) => {
  await boot(page, {
    games_content_list: {
      items: [odin],
      damaged: [{ itemId: 'abc123', message: 'items/abc123.json is not a valid item record: EOF while parsing' }],
    },
    games_content_remove: null,
  });
  await openLibrary(page);

  const damaged = page.getByTestId('library-damaged');
  await expect(damaged).toContainText('A stored record cannot be read');
  await expect(damaged).toContainText('not a valid item record');
  await damaged.getByRole('button', { name: 'Remove' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Remove' }).click();
  await expect(page.getByText('Removed abc123.', { exact: true })).toBeVisible();
});
