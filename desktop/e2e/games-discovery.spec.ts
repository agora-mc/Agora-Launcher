import { test, expect, type Page } from '@playwright/test';
import { expectNoUnmockedCommands, installGamesMock } from './support/games-mock';

/**
 * Setup › Installs (MASTER_SPEC §26.3, §26.14): the installs Agora found, grouped by game, and
 * the game folders the user adds and removes. The table answers the commands the shell needs at
 * boot; the discovery commands themselves are answered by a small script that also records each
 * call, so a test can check what the screen asked for and react to an add or a remove.
 */

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

const skyrimInstall = {
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
};

const skyrim = {
  gameId: 'skyrim-se',
  name: 'Skyrim Special Edition',
  installs: [skyrimInstall],
};

const copyInstall = {
  installId: 'direct-d-games-skyrim-copy',
  store: 'direct',
  name: 'Skyrim Copy',
  location: 'D:/Games/Skyrim Copy',
  storeVersion: null,
  storeBuild: null,
  addOnCount: 0,
  installBytes: null,
  runtime: { status: 'unidentified', reasons: ['The executable could not be read.'] },
};

const COPY_PATH = 'D:/Games/Skyrim Copy';

interface DiscoveryScript {
  /** What the games list shows before anything is added. */
  games: unknown[];
  /** What it shows after a folder is added. Defaults to `games`. */
  gamesAfterAdd?: unknown[];
  /** Folders the scan lists before anything is added. */
  folders?: { path: string; present: boolean }[];
  /** The path the folder picker returns. `null` is a cancelled picker. */
  picked?: string | null;
  /** The folder core accepts when one is added. */
  added?: { path: string; gameId: string; gameName: string };
  /** Commands that reject, with the GamesError the backend would send. */
  rejects?: Record<string, { code: string; message: string; coreCode?: string }>;
}

/**
 * Answers the discovery commands from `script` and records every call on `window`. Runs after
 * the games mock is installed, so `__TAURI_INTERNALS__` already exists.
 */
async function driveDiscovery(page: Page, script: DiscoveryScript) {
  await page.addInitScript((config) => {
    const tauri = (window as unknown as { __TAURI_INTERNALS__: { invoke: (c: string, a?: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__;
    const original = tauri.invoke;
    const calls: { command: string; args: Record<string, unknown> }[] = [];
    (window as unknown as { __discoveryCalls: typeof calls }).__discoveryCalls = calls;
    let added = false;
    let folders: { path: string; present: boolean }[] = config.folders ?? [];
    tauri.invoke = (command: string, args: Record<string, unknown> = {}) => {
      calls.push({ command, args: JSON.parse(JSON.stringify(args)) });
      const rejected = config.rejects?.[command];
      if (rejected) return Promise.reject(rejected);
      switch (command) {
        case 'pick_directory':
          return Promise.resolve(config.picked ?? null);
        case 'games_list':
          // The test turns this on right before it clicks Rescan, so the first load finds Skyrim.
          if ((window as unknown as { __discoveryEmpty?: boolean }).__discoveryEmpty) return Promise.resolve({ games: [] });
          return Promise.resolve({ games: added ? (config.gamesAfterAdd ?? config.games) : config.games });
        case 'games_discover':
          return Promise.resolve({ folders, warnings: [] });
        case 'games_add_folder':
          added = true;
          folders = [{ path: config.added!.path, present: true }];
          return Promise.resolve(config.added);
        case 'games_remove_folder': {
          const path = String(args.path);
          folders = folders.filter((folder) => folder.path !== path);
          return Promise.resolve(path);
        }
        default:
          return original(command, args);
      }
    };
  }, script);
}

async function calls(page: Page): Promise<{ command: string; args: Record<string, unknown> }[]> {
  return page.evaluate(() => (window as unknown as { __discoveryCalls: { command: string; args: Record<string, unknown> }[] }).__discoveryCalls);
}

async function openSetup(page: Page) {
  await page.goto('/');
  await page.getByTestId('game-switcher').getByRole('button', { name: 'Game: Minecraft. Choose a game' }).click();
  await page.getByRole('button', { name: 'Skyrim Special Edition', exact: true }).click();
  await page.getByRole('navigation', { name: 'Main navigation' }).getByRole('button', { name: 'Setup' }).click();
}

test.afterEach(async ({ page }) => {
  await expectNoUnmockedCommands(page);
});

test('the installs are listed by game, and a folder can be added and then shows up', async ({ page }) => {
  await installGamesMock(page, { replies: BOOT_REPLIES });
  await driveDiscovery(page, {
    games: [skyrim],
    gamesAfterAdd: [{ gameId: 'skyrim-se', name: 'Skyrim Special Edition', installs: [skyrimInstall, copyInstall] }],
    picked: COPY_PATH,
    added: { path: COPY_PATH, gameId: 'skyrim-se', gameName: 'Skyrim Special Edition' },
  });
  await openSetup(page);

  const section = page.getByTestId('setup-installs');
  await expect(section.getByRole('heading', { name: 'Installs' })).toBeVisible();
  const card = page.getByTestId('install-card').first();
  await expect(card).toContainText('Skyrim Special Edition');
  await expect(card).toContainText('Steam');
  await expect(card).toContainText('Version 1.6.1170');
  await expect(card).toContainText('D:/Games/Skyrim Special Edition');

  await section.getByRole('button', { name: 'Add a game folder…' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Added' })).toHaveText(
    `Added ${COPY_PATH} as Skyrim Special Edition.`,
  );
  await expect(page.getByTestId('setup-folder-row')).toContainText(COPY_PATH);
  await expect(page.getByTestId('install-card')).toHaveCount(2);

  const added = (await calls(page)).filter((call) => call.command === 'games_add_folder');
  expect(added).toEqual([{ command: 'games_add_folder', args: { path: COPY_PATH } }]);
});

test('cancelling the folder picker adds nothing', async ({ page }) => {
  await installGamesMock(page, { replies: BOOT_REPLIES });
  await driveDiscovery(page, { games: [skyrim], picked: null });
  await openSetup(page);

  await page.getByTestId('setup-installs').getByRole('button', { name: 'Add a game folder…' }).click();
  await expect(page.getByTestId('install-card')).toHaveCount(1);
  expect((await calls(page)).some((call) => call.command === 'games_add_folder')).toBe(false);
});

test('an empty scan says what to do next and offers the folder picker', async ({ page }) => {
  // Setup is reached only while a game is found, so Skyrim is found first; a Rescan then finds
  // nothing, which is the empty state a user can actually reach.
  await installGamesMock(page, { replies: BOOT_REPLIES });
  await driveDiscovery(page, { games: [skyrim] });
  await openSetup(page);
  await expect(page.getByTestId('install-card')).toHaveCount(1);
  await page.evaluate(() => {
    (window as unknown as { __discoveryEmpty: boolean }).__discoveryEmpty = true;
  });
  await page.getByTestId('setup-installs').getByRole('button', { name: 'Rescan' }).click();

  const empty = page.getByTestId('installs-empty');
  await expect(empty).toContainText('Agora did not find this game on this computer.');
  await expect(empty).toContainText('Add a game folder…');
  await expect(page.getByTestId('install-card')).toHaveCount(0);
  await expect(page.getByTestId('setup-folders')).toContainText('None yet.');
});

test('a failed scan shows its error and no list, so it is not read as nothing found', async ({ page }) => {
  await installGamesMock(page, { replies: BOOT_REPLIES });
  await driveDiscovery(page, {
    games: [skyrim],
    rejects: {
      games_discover: {
        code: 'core_failed',
        coreCode: 'ERR_GAME_FOLDERS_UNREADABLE',
        message: 'Your game folders list is damaged. Fix or delete the file. Agora will not guess which folders you added.',
      },
    },
  });
  await openSetup(page);

  const alert = page.getByRole('alert');
  await expect(alert).toContainText('Your game folders list is damaged.');
  await expect(alert.getByRole('button', { name: 'Try again' })).toBeVisible();
  await expect(page.getByTestId('installs-empty')).toHaveCount(0);
  await expect(page.getByTestId('installs-game')).toHaveCount(0);
});

test('removing a folder asks first, and only a confirmed removal removes it', async ({ page }) => {
  await installGamesMock(page, { replies: BOOT_REPLIES });
  await driveDiscovery(page, {
    games: [skyrim],
    folders: [{ path: COPY_PATH, present: true }],
  });
  await openSetup(page);

  const row = page.getByTestId('setup-folder-row');
  await expect(row).toContainText(COPY_PATH);
  await page.getByRole('button', { name: `Stop searching ${COPY_PATH}` }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Agora will forget D:/Games/Skyrim Copy.');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(row).toHaveCount(1);
  expect((await calls(page)).some((call) => call.command === 'games_remove_folder')).toBe(false);

  await page.getByRole('button', { name: `Stop searching ${COPY_PATH}` }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Stop searching', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Stopped searching' })).toContainText('Its files were not touched.');
  await expect(page.getByTestId('setup-folder-row')).toHaveCount(0);
  const removed = (await calls(page)).filter((call) => call.command === 'games_remove_folder');
  expect(removed).toEqual([{ command: 'games_remove_folder', args: { path: COPY_PATH } }]);
});
