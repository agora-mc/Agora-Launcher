import { test, expect, type Page } from '@playwright/test';
import { expectNoUnmockedCommands, installGamesMock } from './support/games-mock';

/**
 * The instances slice (MASTER_SPEC §26.14): the Instances tab's cards, create and delete, and an
 * instance's Overview with its launch checks, deployment choice and launch. Every command these
 * tests reach is answered from the table; anything else fails the test.
 */

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

const grounded = {
  instanceId: 'grounded-1',
  gameId: 'skyrim-se',
  name: 'Grounded',
  store: 'steam',
  version: '1.6.1170',
  pinned: true,
  deployment: null,
  lastLaunchedAt: '2026-10-20 18:30:00',
  running: false,
};

const modded = {
  instanceId: 'modded-2',
  gameId: 'skyrim-se',
  name: 'Modded',
  store: 'steam',
  version: '1.6.1170',
  pinned: false,
  deployment: 'links',
  lastLaunchedAt: null,
  running: false,
};

const groundedDetail = {
  instanceId: 'grounded-1',
  gameId: 'skyrim-se',
  gameName: 'Skyrim Special Edition',
  name: 'Grounded',
  pinned: true,
  store: 'steam',
  version: '1.6.1170',
  build: null,
  baseId: 'skyrim-se_steam_1.6.1170',
  installId: null,
  unpinnedReason: null,
  deployment: null,
  saves: 'shared',
  createdAt: '2026-10-01 10:00:00',
  lastLaunchedAt: '2026-10-20 18:30:00',
  running: false,
};

const extenderFinding = {
  kind: 'framework',
  blocks: true,
  title: 'Script extender',
  detail: 'It needs skse64_1_6_1170.dll, but the game has skse64_1_5_97.dll.',
  why: 'A framework built for another game version often stops the game from starting, or makes it crash.',
  repair: 'Install the extender build for 1.6.1170.',
};

const blockedCheck = {
  instanceId: 'grounded-1',
  checked: true,
  status: 'findings',
  findings: [extenderFinding],
};

const cleanCheck = {
  instanceId: 'grounded-1',
  checked: true,
  status: 'ok',
  findings: [],
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

/** Starts the app on Skyrim's Instances tab with the given command answers. */
async function boot(page: Page, replies: Record<string, unknown>) {
  await installGamesMock(page, {
    replies: { ...BOOT_REPLIES, games_list: { games: [skyrim] }, ...replies },
    settings: { active_game: 'skyrim-se' },
  });
  await page.goto('/');
  await expect(page.getByTestId('game-instances-tab')).toBeVisible();
}

function cardFor(page: Page, name: string) {
  return page.getByTestId('instance-card').filter({ has: page.getByRole('heading', { name, exact: true }) });
}

test.afterEach(async ({ page }) => {
  await expectNoUnmockedCommands(page);
});

test('the Instances tab shows each instance as a card with its runtime, pinning and deployment', async ({ page }) => {
  await boot(page, { games_instance_list: { instances: [grounded, modded] } });

  const cards = page.getByTestId('instance-card');
  await expect(cards).toHaveCount(2);

  await expect(cardFor(page, 'Grounded')).toContainText('steam 1.6.1170');
  await expect(cardFor(page, 'Grounded')).toContainText('Pinned base');
  await expect(cardFor(page, 'Grounded')).toContainText('Automatic');

  await expect(cardFor(page, 'Modded')).toContainText('Runs from install');
  await expect(cardFor(page, 'Modded')).toContainText('Linked files');
  await expect(cardFor(page, 'Modded')).toContainText('Never');
});

test('an empty game says what to do next, and the first instance is one button away', async ({ page }) => {
  await boot(page, { games_instance_list: { instances: [] } });

  await expect(page.getByTestId('instances-empty')).toContainText('No instances yet');
  await expect(page.getByRole('button', { name: 'Create the first instance' })).toBeVisible();
});

test('a list that does not come back says so, and can be tried again', async ({ page }) => {
  // `null` is not a list: the page must show the failure, not "no instances".
  await boot(page, { games_instance_list: null });

  const alert = page.getByRole('alert').filter({ hasText: 'The instances could not be loaded' });
  await expect(alert).toContainText('The instances could not be loaded');
  await expect(alert).toContainText('games_instance_list did not return what the Instances page expects.');
  await expect(page.getByTestId('instances-empty')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Try again' })).toBeVisible();
});

test('an instance opens to its Overview, where a framework that does not match blocks the launch', async ({ page }) => {
  await boot(page, {
    games_instance_list: { instances: [grounded] },
    games_instance_get: groundedDetail,
    games_instance_check: blockedCheck,
  });

  await cardFor(page, 'Grounded').getByRole('button', { name: 'Open' }).click();

  const overview = page.getByTestId('instance-overview');
  await expect(overview.getByRole('heading', { name: 'Grounded', exact: true })).toBeVisible();
  await expect(page.getByTestId('frameworks-status')).toHaveText('Do not match this game version');
  await expect(page.getByTestId('check-finding')).toHaveAttribute('data-blocks', 'true');
  await expect(page.getByTestId('check-finding')).toContainText('Stops the launch');
  await expect(page.getByTestId('check-finding')).toContainText('Install the extender build for 1.6.1170.');
  await expect(page.getByRole('button', { name: 'Launch Grounded' })).toBeEnabled();
  // No deployment recorded means automatic, so that is the choice the page shows as pressed.
  await expect(page.getByRole('button', { name: 'Automatic' })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'Linked files' })).toHaveAttribute('aria-pressed', 'false');
});

test('a clean check says so, and a launch the checks refused starts nothing and offers launch-anyway only under advanced options', async ({ page }) => {
  await boot(page, {
    games_instance_list: { instances: [grounded] },
    games_instance_get: groundedDetail,
    games_instance_check: cleanCheck,
    games_instance_launch: {
      status: 'refused',
      pid: null,
      deployment: null,
      notice: null,
      findings: [extenderFinding],
      reason: null,
      next: null,
    },
  });

  await cardFor(page, 'Grounded').getByRole('button', { name: 'Open' }).click();
  await expect(page.getByTestId('frameworks-status')).toHaveText('Match this game version');

  await page.getByRole('button', { name: 'Launch Grounded' }).click();
  await expect(page.getByRole('alert').filter({ hasText: 'The launch was stopped by these checks' })).toContainText('Nothing was started.');
  await expect(page.getByRole('checkbox', { name: /Launch anyway/ })).toHaveCount(0);

  await page.getByRole('button', { name: 'Advanced options' }).click();
  const anyway = page.getByRole('checkbox', { name: /Launch anyway/ });
  await expect(anyway).not.toBeChecked();
  await expect(page.getByText('Use this only when you know why a check is wrong.')).toBeVisible();
});

test('when the virtual file system will not start, the page asks before it runs from linked files', async ({ page }) => {
  await boot(page, {
    games_instance_list: { instances: [grounded] },
    games_instance_get: groundedDetail,
    games_instance_check: cleanCheck,
    games_instance_launch: {
      status: 'vfs_unavailable',
      pid: null,
      deployment: null,
      notice: null,
      findings: [],
      reason: 'the virtual file system driver is not installed',
      next: 'links',
    },
  });

  await cardFor(page, 'Grounded').getByRole('button', { name: 'Open' }).click();
  await page.getByRole('button', { name: 'Launch Grounded' }).click();

  const question = page.getByTestId('vfs-question');
  await expect(question).toContainText('The virtual file system could not start.');
  await expect(question).toContainText('the virtual file system driver is not installed');
  await expect(question).toContainText('Nothing has been changed yet.');
  await expect(question).toContainText('Linked files cannot catch writes');
  await expect(question.getByRole('button', { name: 'Run once from linked files' })).toBeVisible();
  await expect(question.getByRole('button', { name: 'Always run from linked files' })).toBeVisible();
});

test('deleting an instance asks first, and cancelling keeps it', async ({ page }) => {
  await boot(page, { games_instance_list: { instances: [grounded] } });

  await page.getByRole('button', { name: 'Delete Grounded' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Delete Grounded?');
  await expect(dialog).toContainText('The game install and the pinned base are kept.');
  await dialog.getByRole('button', { name: 'Cancel' }).click();

  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByTestId('instance-card')).toHaveCount(1);
});

test('creating an instance needs a name, explains how it runs, and opens the new instance', async ({ page }) => {
  await boot(page, {
    games_instance_list: { instances: [] },
    games_instance_create: {
      instanceId: 'grounded-1',
      name: 'Grounded',
      base: 'pinned',
      reusedBase: true,
      baseId: 'skyrim-se_steam_1.6.1170',
      unpinnedReason: null,
    },
    games_instance_get: groundedDetail,
    games_instance_check: cleanCheck,
  });

  await page.getByRole('button', { name: 'Create the first instance' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('heading', { name: 'New instance of Skyrim Special Edition' })).toBeVisible();
  await expect(dialog).toContainText('Automatic is the default');
  await expect(dialog).toContainText('Virtual file system.');
  await expect(dialog.getByRole('button', { name: 'Create instance' })).toBeDisabled();

  // A name of only spaces is still no name.
  await dialog.getByLabel('Name').fill('   ');
  await expect(dialog.getByRole('button', { name: 'Create instance' })).toBeDisabled();

  await dialog.getByLabel('Name').fill('Grounded');
  await expect(dialog.getByRole('button', { name: 'Create instance' })).toBeEnabled();
  await dialog.getByRole('button', { name: 'Create instance' }).click();

  await expect(page.getByTestId('instance-overview').getByRole('heading', { name: 'Grounded', exact: true })).toBeVisible();
});

test('a launch that starts names the process, how it runs, and what the page does about it', async ({ page }) => {
  await boot(page, {
    games_instance_list: { instances: [grounded] },
    games_instance_get: groundedDetail,
    games_instance_check: cleanCheck,
    games_instance_launch: {
      status: 'launched',
      pid: 4242,
      deployment: 'links',
      notice: 'The virtual file system is not set up, so this launch uses linked files.',
      findings: [],
      reason: null,
      next: null,
    },
  });

  await cardFor(page, 'Grounded').getByRole('button', { name: 'Open' }).click();
  await page.getByRole('button', { name: 'Launch Grounded' }).click();

  const result = page.getByRole('status').filter({ hasText: 'The game is starting (process 4242)' });
  await expect(result).toContainText('Agora will put your files back when it closes.');
  await expect(result).toContainText('Running from linked files.');
  await expect(result).toContainText('The virtual file system is not set up, so this launch uses linked files.');
  await expect(page.getByRole('button', { name: 'Launch Grounded' })).toBeVisible();
});
