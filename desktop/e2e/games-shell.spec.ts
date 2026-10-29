import { test, expect, type Page } from '@playwright/test';
import { expectNoUnmockedCommands, installGamesMock, storedSetting } from './support/games-mock';

/**
 * The multi-game shell (MASTER_SPEC §26.14): the header switcher, the per-game tabs, and the
 * choice that is remembered. Every command these tests reach is answered from the table in
 * `support/games-mock.ts`; anything else fails the test.
 */

// Living Background is on by default (ambience is enabled until the user turns it off).
const MINECRAFT_NAV = [
  'Home',
  'Browse',
  'My Instances',
  'Community Governance',
  'Help & Guide',
  'Field Guide',
  'Living Background',
  'The Agora Difference',
  'Settings',
];
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

async function boot(page: Page, games: unknown, settings: Record<string, unknown> = {}) {
  await installGamesMock(page, {
    replies: { ...BOOT_REPLIES, games_list: games },
    settings,
  });
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

test('the switcher stays hidden and the Minecraft sidebar is unchanged with no other game', async ({ page }) => {
  await boot(page, { games: [] });
  await page.goto('/');

  await expect(page.getByTestId('game-switcher')).toHaveCount(0);
  await expect.poll(() => navLabels(page)).toEqual(MINECRAFT_NAV);
});

test('a discovery that fails to answer also keeps the switcher hidden', async ({ page }) => {
  // `null` is a reply that is not a list of games; the shell must not read it as "none found" and
  // must not offer anything either.
  await boot(page, null);
  await page.goto('/');

  await expect(sidebarNav(page).getByRole('button', { name: 'Browse' })).toBeVisible();
  await expect(page.getByTestId('game-switcher')).toHaveCount(0);
});

test('the switcher is shown once Skyrim is found, and lists Minecraft first', async ({ page }) => {
  await boot(page, { games: [skyrim] });
  await page.goto('/');

  const switcher = page.getByTestId('game-switcher');
  await expect(switcher).toBeVisible();
  await switcher.getByRole('button', { name: 'Game: Minecraft. Choose a game' }).click();
  const options = switcher.getByRole('group', { name: 'Games' }).getByRole('button');
  await expect(options).toHaveText(['Minecraft', 'Skyrim Special Edition']);
});

test('switching to Skyrim swaps the sidebar to its tabs and back again', async ({ page }) => {
  await boot(page, { games: [skyrim] });
  await page.goto('/');

  await page.getByTestId('game-switcher').getByRole('button', { name: 'Game: Minecraft. Choose a game' }).click();
  await page.getByRole('button', { name: 'Skyrim Special Edition', exact: true }).click();

  await expect(page.getByTestId('game-switcher').getByRole('button', { name: /Game: Skyrim Special Edition/ })).toBeVisible();
  await expect.poll(() => navLabels(page))
    .toEqual(GAME_NAV);
  await expect(page.getByTestId('game-slot-placeholder').first()).toHaveAttribute('data-slice', 'instances');

  await page.getByTestId('game-switcher').getByRole('button', { name: /Game: Skyrim Special Edition/ }).click();
  await page.getByTestId('game-switcher').getByRole('button', { name: 'Minecraft', exact: true }).click();

  await expect.poll(() => navLabels(page))
    .toEqual(MINECRAFT_NAV);
  await expect(page.getByTestId('game-switcher').getByRole('button', { name: 'Game: Minecraft. Choose a game' })).toBeVisible();
});

test('the chosen game is still chosen after a reload, and after the app starts again', async ({ page }) => {
  await boot(page, { games: [skyrim] });
  await page.goto('/');

  await page.getByTestId('game-switcher').getByRole('button', { name: 'Game: Minecraft. Choose a game' }).click();
  await page.getByRole('button', { name: 'Skyrim Special Edition', exact: true }).click();
  await expect.poll(() => navLabels(page))
    .toEqual(GAME_NAV);

  await page.reload();
  await expect.poll(() => navLabels(page))
    .toEqual(GAME_NAV);

  // A fresh start is a navigation to the bare app address, which carries no history state. The app
  // must open the game the user last chose, not Minecraft's Home.
  await page.goto('/');
  await expect.poll(() => navLabels(page))
    .toEqual(GAME_NAV);
});

test('a remembered game that is no longer found opens Minecraft', async ({ page }) => {
  await boot(page, { games: [] }, { active_game: 'skyrim-se' });
  await page.goto('/');

  await expect(sidebarNav(page).getByRole('button', { name: 'Browse' })).toBeVisible();
  await expect(page.getByTestId('game-switcher')).toHaveCount(0);
});

test('a failed discovery shows Minecraft and keeps the chosen game for next time', async ({ page }) => {
  await boot(page, null, { active_game: 'skyrim-se' });
  await page.goto('/');

  await expect.poll(() => navLabels(page)).toEqual(MINECRAFT_NAV);
  await sidebarNav(page).getByRole('button', { name: 'Field Guide' }).click();
  await expect(page.getByTestId('game-switcher')).toHaveCount(0);
  expect(await storedSetting(page, 'active_game')).toBe('skyrim-se');
});
