import { test, expect, type Page } from '@playwright/test';
import { expectNoUnmockedCommands, installGamesMock } from './support/games-mock';

/**
 * The import slice's screens on a non-Minecraft game: the Import tab (Mod Organizer 2), the
 * Library's curated items, and the Setup tab's recovery section. Every command is answered from
 * a table; anything else fails the test (see `support/games-mock.ts`).
 *
 * Commands whose reply depends on the request (a dry run and a real import share one command) are
 * answered by `answerWith`, which sits in front of the mock and matches on the request's fields.
 */

const GAME_NAV = ['Instances', 'Library', 'Import', 'Setup', 'Settings'];

const skyrimInstall = {
  installId: 'steam-489830',
  store: 'steam',
  name: 'Skyrim Special Edition',
  location: 'D:/Games/Skyrim Special Edition',
  storeVersion: '1.6.1170',
  storeBuild: null,
  addOnCount: 0,
  runtime: {
    status: 'identified',
    runtime: { game: 'skyrim-se', store: 'steam', version: '1.6.1170', build: null },
    source: 'executable',
  },
};

const skyrim = {
  gameId: 'skyrim-se',
  name: 'Skyrim Special Edition',
  installs: [skyrimInstall],
};

/** The boot commands the shell answers with empty data. Shared by every test here. */
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
};

const setup = {
  iniPath: 'D:/Setups/MO2/ModOrganizer.ini',
  gameName: 'Skyrim Special Edition',
  profiles: ['Default'],
  modCount: 2,
  foundBy: 'the usual place (ModOrganizer folder)',
};

const plan = {
  kind: 'plan',
  setup: setup.iniPath,
  profile: 'Default',
  mo2GameName: 'Skyrim Special Edition',
  gameId: 'skyrim-se',
  installLocation: 'D:/Games/Skyrim Special Edition',
  store: 'steam',
  instanceName: 'Default (MO2 import)',
  mods: [
    {
      line: 1,
      folder: 'SkyUI',
      enabled: true,
      files: 12,
      bytes: 2048,
      provenance: 'claims Nexus mod 3863, not verified',
      pluginFiles: ['SkyUI_SE.esp'],
      metaIniExcluded: true,
    },
    {
      line: 2,
      folder: 'Old Patch',
      enabled: false,
      files: 3,
      bytes: 512,
      provenance: 'no meta.ini',
      pluginFiles: [],
      metaIniExcluded: false,
    },
  ],
  modsEnabled: 1,
  modsDisabled: 1,
  files: 15,
  bytes: 2560,
  metaIniExcluded: 1,
  separators: 0,
  unmanaged: [],
  problems: [{ line: 9, entry: 'Ghost Mod', reason: 'the mod’s folder is missing from the mods folder; skipped' }],
  overwrite: { present: false, files: 0, bytes: 0, generated: [], restFiles: 0 },
  plugins: { kept: true, listed: 2, active: 1, locked: [], implicitSkipped: [], notes: [] },
  localSettings: false,
  inis: [],
  saves: { local: false, files: 0, bytes: 0, note: '' },
  warnings: [],
};

const importRun = {
  kind: 'run',
  instanceId: 'inst-9',
  instanceName: 'Default (MO2 import)',
  baseId: 'base-1',
  baseBuilt: true,
  storedMods: 2,
  contentLayers: 2,
  disabledLayers: 1,
  duplicates: [],
  generatedLayers: [],
  pluginsWritten: 2,
  pluginFindings: [],
  inisCopied: [],
  savesCopied: 0,
  savesSkippedExisting: 0,
  savesFolder: null,
  nextSteps: ['line 9: Ghost Mod was not imported: the mod’s folder is missing from the mods folder; skipped'],
};

const catalogEntry = {
  id: 'skse64',
  name: 'SKSE64',
  author: 'ianpatt',
  description: 'Script extender for Skyrim.',
  downloadStrategy: 'github_release',
  status: 'active',
  pageUrl: null,
  compatibility: [
    {
      stores: ['steam'],
      gameVersions: ['1.6.1170'],
      requires: [],
      asset: 'skse64_*.7z',
    },
  ],
};

const catalogListing = {
  entries: [catalogEntry],
  instances: [{ instanceId: 'inst-1', name: 'Skyrim Main' }],
};

const catalogPlan = {
  itemId: 'skse64',
  itemName: 'SKSE64',
  instanceId: 'inst-1',
  source: 'GitHub ianpatt/skse64 release 2.2.6',
  fileName: 'skse64_2_2_6.7z',
  release: '2.2.6',
  size: 2097152,
  frameworks: [],
  warnings: [],
};

const verifiedHash = {
  sha256: 'ab'.repeat(32),
  verified: true,
  basis: 'GitHub published this SHA-256 for the asset',
  confirmedPast: null,
};

const unverifiedHash = {
  sha256: 'cd'.repeat(32),
  verified: false,
  basis: 'no published hash; the file is recorded as it is',
  confirmedPast: null,
};

const userFilesSession = {
  gameId: 'skyrim-se',
  store: 'steam',
  instanceId: 'inst-1',
  running: false,
  files: [
    { realPath: 'C:/Users/me/Documents/My Games/Skyrim Special Edition/Skyrim.ini', swapped: true },
  ],
};

interface Rule {
  command: string;
  /** Matched against the request's fields (or the arguments, for commands without `request`). */
  match?: Record<string, unknown>;
  reply?: unknown;
  error?: { code: string; message: string };
}

/**
 * Answers commands by rule, in front of the games mock. The first rule whose command and fields
 * match answers; anything else goes to the mock, which fails unmocked commands.
 */
async function answerWith(page: Page, rules: Rule[]) {
  await page.addInitScript((list: Rule[]) => {
    const internals = (window as unknown as {
      __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
    }).__TAURI_INTERNALS__;
    const inner = internals.invoke;
    internals.invoke = (command: string, args: Record<string, unknown> = {}) => {
      const body = (args.request ?? args) as Record<string, unknown>;
      const rule = list.find(
        (candidate) =>
          candidate.command === command
          && Object.entries(candidate.match ?? {}).every(([key, value]) => body[key] === value),
      );
      if (!rule) return inner(command, args);
      return rule.error ? Promise.reject(rule.error) : Promise.resolve(rule.reply);
    };
  }, rules);
}

async function boot(page: Page, rules: Rule[], replies: Record<string, unknown>) {
  await installGamesMock(page, {
    replies: { ...BOOT_REPLIES, games_list: { games: [skyrim] }, ...replies },
    settings: { active_game: 'skyrim-se' },
  });
  await answerWith(page, rules);
  await page.goto('/');
  await expect.poll(() => navLabels(page)).toEqual(GAME_NAV);
}

function sidebarNav(page: Page) {
  return page.getByRole('navigation', { name: 'Main navigation' });
}

async function navLabels(page: Page): Promise<string[]> {
  return (await sidebarNav(page).getByRole('button').allTextContents()).map((text) => text.trim());
}

test.afterEach(async ({ page }) => {
  await expectNoUnmockedCommands(page);
});

test.describe('Import tab', () => {
  test('finds a setup, previews the profile, and imports it after confirming', async ({ page }) => {
    await boot(
      page,
      [
        { command: 'games_import_mo2', match: { dryRun: true }, reply: plan },
        { command: 'games_import_mo2', match: { dryRun: false }, reply: importRun },
      ],
      { games_import_mo2_scan: { setups: [setup], warnings: [] } },
    );

    await sidebarNav(page).getByRole('button', { name: 'Import' }).click();
    await expect(page.getByRole('heading', { name: 'Import from Mod Organizer 2' })).toBeVisible();
    await page.getByRole('list', { name: 'Setups found' }).getByRole('button', { name: 'Use this setup' }).click();

    await page.getByRole('button', { name: 'Preview the import' }).click();
    const preview = page.getByTestId('import-plan');
    await expect(preview.getByTestId('import-plan-mods')).toHaveText('1 enabled mod, 1 disabled mod');
    // Nothing is skipped silently: the skipped line and its reason are listed.
    await expect(page.getByTestId('import-plan-problems')).toContainText('Ghost Mod');
    await expect(page.getByTestId('import-plan-problems')).toContainText('folder is missing');

    await page.getByRole('button', { name: 'Import as a new instance' }).click();
    await page.getByRole('button', { name: 'Import profile', exact: true }).click();

    const done = page.getByTestId('import-done');
    await expect(done).toContainText('Imported as “Default (MO2 import)”');
    await expect(done).toContainText('2 mod(s) stored');
    await expect(done).toContainText('Still to do');
  });

  test('says so when no setup is found, and offers a folder to choose', async ({ page }) => {
    await boot(page, [], { games_import_mo2_scan: { setups: [], warnings: [] } });

    await sidebarNav(page).getByRole('button', { name: 'Import' }).click();
    await expect(page.getByTestId('import-empty')).toContainText('No Mod Organizer 2 setups were found');
    await expect(page.getByRole('button', { name: 'Choose a setup folder…' })).toBeVisible();
  });

  test('a setup search that fails shows the reason, not an empty list', async ({ page }) => {
    await boot(
      page,
      [{ command: 'games_import_mo2_scan', error: { code: 'core_failed', message: 'The drives could not be read.' } }],
      {},
    );

    await sidebarNav(page).getByRole('button', { name: 'Import' }).click();
    await expect(page.getByRole('alert')).toContainText('The drives could not be read.');
    await expect(page.getByTestId('import-empty')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Look again' })).toBeEnabled();
  });

  test('a folder picker that fails says so instead of looking like a cancelled pick', async ({ page }) => {
    await boot(
      page,
      [{ command: 'pick_directory', error: { code: 'picker_failed', message: 'The folder picker could not open.' } }],
      { games_import_mo2_scan: { setups: [], warnings: [] } },
    );

    await sidebarNav(page).getByRole('button', { name: 'Import' }).click();
    await expect(page.getByTestId('import-empty')).toBeVisible();
    await page.getByRole('button', { name: 'Choose a setup folder…' }).click();
    await expect(page.getByRole('alert')).toContainText('The folder picker could not open.');
  });

  test('a profile from another game is previewed but cannot be imported here', async ({ page }) => {
    const fallout = {
      ...plan,
      gameId: 'fallout-4',
      mo2GameName: 'Fallout 4',
      installLocation: 'D:/Games/Fallout 4',
      instanceName: 'Default (MO2 import)',
    };
    await boot(
      page,
      [{ command: 'games_import_mo2', match: { dryRun: true }, reply: fallout }],
      { games_import_mo2_scan: { setups: [setup], warnings: [] } },
    );

    await sidebarNav(page).getByRole('button', { name: 'Import' }).click();
    await page.getByRole('button', { name: 'Use this setup' }).click();
    await page.getByRole('button', { name: 'Preview the import' }).click();

    await expect(page.getByTestId('import-plan').getByRole('alert')).toContainText('not this game');
    await expect(page.getByRole('button', { name: 'Import as a new instance' })).toBeDisabled();
  });
});

test.describe('Library curated items', () => {
  test('previews an entry, then installs it and says how its hash was checked', async ({ page }) => {
    await boot(
      page,
      [
        { command: 'games_catalog_install', match: { dryRun: true }, reply: { state: 'dry_run', ...catalogPlan } },
        {
          command: 'games_catalog_install',
          match: { dryRun: false },
          reply: {
            state: 'placed',
            ...catalogPlan,
            contentItemId: 'content-1',
            mountPath: 'Data',
            storedSize: 2097152,
            hash: verifiedHash,
          },
        },
      ],
      { games_catalog_list: catalogListing },
    );

    await sidebarNav(page).getByRole('button', { name: 'Library' }).click();
    const row = page.getByTestId('catalog-entry');
    await expect(row).toContainText('SKSE64');

    await row.getByRole('button', { name: 'Preview' }).click();
    await expect(page.getByTestId('catalog-preview')).toContainText('Nothing has been downloaded');

    await row.getByRole('button', { name: 'Install', exact: true }).click();
    await expect(page.getByTestId('catalog-placed')).toContainText('Installed SKSE64 into Data.');
    await expect(page.getByTestId('catalog-hash')).toContainText('Verified: the file matches a hash on record.');
    await expect(page.getByTestId('catalog-hash')).toContainText('GitHub published this SHA-256');
  });

  test('an unverified file is never shown as verified', async ({ page }) => {
    await boot(
      page,
      [
        {
          command: 'games_catalog_install',
          match: { dryRun: false },
          reply: {
            state: 'placed',
            ...catalogPlan,
            contentItemId: 'content-2',
            mountPath: 'Data',
            storedSize: 2097152,
            hash: unverifiedHash,
          },
        },
      ],
      { games_catalog_list: catalogListing },
    );

    await sidebarNav(page).getByRole('button', { name: 'Library' }).click();
    await page.getByTestId('catalog-entry').getByRole('button', { name: 'Install', exact: true }).click();
    await expect(page.getByTestId('catalog-hash')).toContainText('Not verified: no published hash');
    await expect(page.getByTestId('catalog-hash')).not.toContainText('Verified:');
  });

  test('an archive with an installer opens the FOMOD wizard for that archive', async ({ page }) => {
    await boot(
      page,
      [
        {
          command: 'games_catalog_install',
          match: { dryRun: false },
          reply: {
            state: 'needs_installer',
            ...catalogPlan,
            contentItemId: 'content-3',
            storedSize: 2097152,
            hash: verifiedHash,
            reason: 'The archive has a FOMOD installer.',
            topLevel: ['fomod', 'Data'],
          },
        },
      ],
      {
        games_catalog_list: catalogListing,
        // The wizard reads its installer when it opens; an empty step list is enough to mount it.
        games_fomod_show: {
          installer: {
            itemId: 'content-3',
            title: 'SKSE64',
            author: null,
            version: null,
            description: null,
            requires: null,
            alwaysInstalledCount: 0,
            conditionalInstallCount: 0,
            steps: [],
          },
          suggestedChoices: null,
        },
      },
    );

    await sidebarNav(page).getByRole('button', { name: 'Library' }).click();
    await page.getByTestId('catalog-entry').getByRole('button', { name: 'Install', exact: true }).click();
    await expect(page.getByTestId('catalog-needs-installer')).toContainText('has an installer with questions');
    // The content slice's real wizard is mounted in place, in its own dialog.
    await expect(page.getByTestId('fomod-wizard')).toBeVisible();
  });

  test('an empty catalog and an instance-less game each say what to do next', async ({ page }) => {
    await boot(page, [], { games_catalog_list: { entries: [], instances: [] } });

    await sidebarNav(page).getByRole('button', { name: 'Library' }).click();
    await expect(page.getByTestId('catalog-empty')).toContainText('no curated items');
    await expect(page.getByTestId('catalog-no-instance')).toContainText('Create an instance');
  });

  test('a catalog that fails to load says so and can be tried again', async ({ page }) => {
    await boot(
      page,
      [{ command: 'games_catalog_list', error: { code: 'bad_game', message: 'Agora does not support that game.' } }],
      {},
    );

    await sidebarNav(page).getByRole('button', { name: 'Library' }).click();
    await expect(page.getByRole('alert')).toContainText('could not load the curated items');
    await expect(page.getByRole('alert')).toContainText('Agora does not support that game.');
    await expect(page.getByTestId('catalog-empty')).toHaveCount(0);
  });
});

test.describe('Setup recovery', () => {
  test('lists a swapped session and puts its files back after confirming', async ({ page }) => {
    await boot(
      page,
      [
        {
          command: 'games_user_files_restore',
          reply: {
            gameId: 'skyrim-se',
            store: 'steam',
            instanceId: 'inst-1',
            files: [
              {
                realPath: 'C:/Users/me/Documents/My Games/Skyrim Special Edition/Skyrim.ini',
                changed: true,
                originalSha256: 'ef'.repeat(32),
                finalSha256: 'ef'.repeat(32),
              },
            ],
          },
        },
      ],
      { games_user_files_status: { sessions: [userFilesSession] } },
    );

    await sidebarNav(page).getByRole('button', { name: 'Setup' }).click();
    const session = page.getByTestId('recovery-session');
    await expect(session).toContainText('steam copy of skyrim-se');
    await expect(session).toContainText('1 file(s), 1 swapped');

    await session.getByRole('button', { name: 'Put the files back' }).click();
    await page.getByRole('button', { name: 'Restore files', exact: true }).click();

    const restored = page.getByTestId('recovery-restored');
    await expect(restored).toContainText('Put back 1 file(s) from the instance inst-1.');
    await expect(restored).toContainText('Changed since the swap: C:/Users/me/Documents/My Games/Skyrim Special Edition/Skyrim.ini');
  });

  test('a session the game is still running on cannot be restored', async ({ page }) => {
    await boot(
      page,
      [],
      { games_user_files_status: { sessions: [{ ...userFilesSession, running: true }] } },
    );

    await sidebarNav(page).getByRole('button', { name: 'Setup' }).click();
    const session = page.getByTestId('recovery-session');
    await expect(session).toContainText('Game is running');
    await expect(session.getByRole('button', { name: 'Put the files back' })).toHaveCount(0);
  });

  test('no swapped sessions says nothing needs recovering', async ({ page }) => {
    await boot(page, [], { games_user_files_status: { sessions: [] } });

    await sidebarNav(page).getByRole('button', { name: 'Setup' }).click();
    await expect(page.getByTestId('recovery-empty')).toContainText('Nothing needs recovering');
  });

  test('a restore that fails shows the reason and leaves the session in place', async ({ page }) => {
    await boot(
      page,
      [{ command: 'games_user_files_restore', error: { code: 'core_failed', message: 'The backup no longer matches.' } }],
      { games_user_files_status: { sessions: [userFilesSession] } },
    );

    await sidebarNav(page).getByRole('button', { name: 'Setup' }).click();
    await page.getByTestId('recovery-session').getByRole('button', { name: 'Put the files back' }).click();
    await page.getByRole('button', { name: 'Restore files', exact: true }).click();

    await expect(page.getByTestId('recovery-session').getByRole('alert')).toContainText('The backup no longer matches.');
    await expect(page.getByTestId('recovery-session')).toContainText('steam copy of skyrim-se');
  });
});
