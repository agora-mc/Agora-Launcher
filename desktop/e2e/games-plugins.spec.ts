import { test, expect, type Page } from '@playwright/test';
import { expectNoUnmockedCommands, installGamesMock } from './support/games-mock';

/**
 * The Plugins and INI & saves tabs of a non-Minecraft game's instance (MASTER_SPEC §26.6, §26.7).
 * Every command these tests reach is answered from the table; anything else fails the test.
 *
 * Replies are fixed per test, so each test that changes something answers with the list as it is
 * after the change, which is what the real command returns.
 */

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

const INSTANCE_ID = 'skyrim-test-1';

/** The boot commands the shell answers with empty data. The same set the shell spec uses. */
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
  games_list: { games: [{ gameId: 'skyrim-se', name: 'Skyrim Special Edition', installs: [skyrimInstall] }] },
};

const SKYRIM_MASTER = 'Skyrim.esm';

/** The save choice reply every INI & saves test needs, since the tab reads both sections. */
const SHARED_SAVES = {
  instanceId: INSTANCE_ID,
  choice: 'shared',
  inUse: { path: 'C:/Shared', exists: true, saves: 0, newest: null },
  other: { path: 'C:/Own', exists: false, saves: 0, newest: null },
  settingFile: 'Skyrim.ini',
  setting: 'General/sSaveFolder',
  settingValue: null,
  settingSource: 'nothing',
};

function plugin(overrides: Record<string, unknown>) {
  return {
    position: 1,
    name: 'Mod.esp',
    implicit: false,
    active: true,
    present: true,
    managed: false,
    locked: false,
    headerRead: true,
    master: false,
    light: false,
    masters: [],
    headerError: null,
    ...overrides,
  };
}

const alwaysLoaded = [
  plugin({ position: 1, name: SKYRIM_MASTER, implicit: true, master: true }),
  plugin({ position: 2, name: 'Update.esm', implicit: true, master: true }),
];

const skyui = plugin({ position: 3, name: 'SkyUI.esp', managed: true, masters: [SKYRIM_MASTER] });
const requiem = plugin({ position: 4, name: 'Requiem.esp', active: false, masters: [SKYRIM_MASTER] });

const orderFinding = {
  message: "'SkyUI.esp' loads before its master 'Skyrim.esm'.",
  refusesLaunch: false,
  plugins: ['SkyUI.esp'],
};

const listed = {
  supported: true,
  rulesApply: true,
  listExists: true,
  rows: [...alwaysLoaded, skyui, requiem],
  findings: [orderFinding],
};

async function openPage(
  page: Page,
  tabPage: 'plugins' | 'ini-saves',
  replies: Record<string, unknown>,
  fail?: { command: string; error: unknown },
) {
  await installGamesMock(page, {
    replies: { ...BOOT_REPLIES, ...replies },
    settings: { active_game: 'skyrim-se' },
  });
  // Added after the mock, so the failing wrapper sees the mock's invoke and not the other way round.
  if (fail) await failCommand(page, fail.command, fail.error);
  await page.goto('/');
  // The app opens an instance page from its saved destination, the way it restores one after a reload.
  await page.evaluate(
    (dest) => window.history.pushState({ __agora: dest, __agoraDepth: 1 }, ''),
    { type: 'game-instance', gameId: 'skyrim-se', instanceId: INSTANCE_ID, page: tabPage },
  );
  await page.reload();
}

/** Makes one command reject, the way a failed backend call does, without touching the other replies. */
async function failCommand(page: Page, command: string, error: unknown) {
  await page.addInitScript(
    ({ command, error }) => {
      const internals = (window as unknown as {
        __TAURI_INTERNALS__: { invoke: (name: string, args?: unknown) => Promise<unknown> };
      }).__TAURI_INTERNALS__;
      const original = internals.invoke.bind(internals);
      internals.invoke = (name: string, args?: unknown) =>
        name === command ? Promise.reject(error) : original(name, args);
    },
    { command, error },
  );
}

test.afterEach(async ({ page }) => {
  await expectNoUnmockedCommands(page);
});

test.describe('Plugins tab', () => {
  test('shows the load order, the problem with it, and the plugins next to it', async ({ page }) => {
    await openPage(page, 'plugins', { games_plugins_list: listed });

    const rows = page.getByTestId('plugin-row');
    await expect(rows).toHaveCount(4);
    await expect(rows.nth(0)).toHaveAttribute('data-plugin', SKYRIM_MASTER);
    await expect(rows.nth(2)).toHaveAttribute('data-plugin', 'SkyUI.esp');
    await expect(page.getByTestId('plugins-findings')).toContainText("'SkyUI.esp' loads before its master 'Skyrim.esm'.");
    // The finding is also written on the row it concerns.
    await expect(rows.nth(2)).toContainText("loads before its master");
    await expect(rows.nth(0)).toContainText('Always loaded');
  });

  test('turning a plugin off shows the list as the backend returns it', async ({ page }) => {
    const off = { ...skyui, active: false };
    await openPage(page, 'plugins', {
      games_plugins_list: listed,
      games_plugins_set_active: {
        changed: true,
        plugins: { ...listed, rows: [...alwaysLoaded, off, requiem] },
      },
    });

    await page.getByRole('switch', { name: 'SkyUI.esp enabled' }).click();
    await expect(page.getByRole('status')).toHaveText('SkyUI.esp is now off.');
    await expect(page.getByRole('switch', { name: 'SkyUI.esp enabled' })).toHaveAttribute('aria-checked', 'false');
  });

  test('the always-loaded plugins and the one above a plugin rule out moving it', async ({ page }) => {
    await openPage(page, 'plugins', { games_plugins_list: listed });

    // Update.esm is always loaded and sits above SkyUI.esp, so SkyUI.esp cannot move up.
    await expect(page.getByRole('button', { name: 'Move SkyUI.esp up' })).toBeDisabled();
    await expect(page.getByRole('button', { name: 'Move SkyUI.esp down' })).toBeEnabled();
    await expect(page.getByRole('switch', { name: `${SKYRIM_MASTER} enabled` })).toBeDisabled();
  });

  test('moving a plugin up shows its new place', async ({ page }) => {
    const moved = { ...requiem, position: 3 };
    const reordered = { ...skyui, position: 4 };
    await openPage(page, 'plugins', {
      games_plugins_list: listed,
      games_plugins_move: {
        report: { plugin: 'Requiem.esp', from: 4, to: 3, written: true },
        plugins: { ...listed, rows: [...alwaysLoaded, moved, reordered] },
      },
    });

    await page.getByRole('button', { name: 'Move Requiem.esp up' }).click();
    await expect(page.getByRole('status')).toHaveText('Requiem.esp moved to position 3.');
    await expect(page.getByTestId('plugin-row').nth(2)).toHaveAttribute('data-plugin', 'Requiem.esp');
  });

  test('a locked plugin cannot be moved until it is unlocked', async ({ page }) => {
    const locked = { ...skyui, locked: true };
    await openPage(page, 'plugins', {
      games_plugins_list: listed,
      games_plugins_set_locked: {
        changed: true,
        plugins: { ...listed, rows: [...alwaysLoaded, locked, requiem] },
      },
    });

    await page.getByRole('button', { name: 'Lock SkyUI.esp' }).click();
    await expect(page.getByRole('status')).toHaveText('SkyUI.esp is locked.');
    await expect(page.getByRole('button', { name: 'Unlock SkyUI.esp' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Move SkyUI.esp down' })).toBeDisabled();
  });

  test('sorting shows the moves first, and only applies them when asked', async ({ page }) => {
    const sortReport = {
      moves: [{ plugin: 'SkyUI.esp', from: 3, to: 4 }],
      blocked: [],
      written: true,
    };
    await openPage(page, 'plugins', {
      games_plugins_list: listed,
      games_plugins_sort: {
        report: sortReport,
        plugins: { ...listed, rows: [...alwaysLoaded, requiem, skyui], findings: [] },
      },
    });

    await page.getByRole('button', { name: 'Sort by masters…' }).click();
    const dialog = page.getByRole('dialog', { name: 'Sort plugins by masters' });
    await expect(dialog).toContainText('SkyUI.esp: position 3 to 4');
    await dialog.getByRole('button', { name: 'Apply 1 move' }).click();
    await expect(page.getByRole('status')).toHaveText('Sorted. 1 plugin moved.');
    await expect(page.getByRole('dialog')).toHaveCount(0);
  });

  test('a list with no plugins yet says what to do', async ({ page }) => {
    await openPage(page, 'plugins', {
      games_plugins_list: { supported: true, rulesApply: true, listExists: true, rows: [], findings: [] },
    });

    await expect(page.getByText("No plugins are in this instance's list yet.")).toBeVisible();
    await expect(page.getByRole('button', { name: 'Check again' })).toHaveCount(0);
  });

  test('an instance with no list yet is told when one appears', async ({ page }) => {
    await openPage(page, 'plugins', {
      games_plugins_list: { supported: true, rulesApply: true, listExists: false, rows: [], findings: [] },
    });

    await expect(page.getByText('This instance has no plugin list yet.')).toBeVisible();
  });

  test('a game with no plugin list says so instead of showing an empty list', async ({ page }) => {
    await openPage(page, 'plugins', {
      games_plugins_list: { supported: false, rulesApply: false, listExists: false, rows: [], findings: [] },
    });

    await expect(page.getByText('This game does not keep a plugin list')).toBeVisible();
    await expect(page.getByTestId('plugin-row')).toHaveCount(0);
  });

  test('a failed read shows the backend message and offers a retry', async ({ page }) => {
    await openPage(
      page,
      'plugins',
      { games_plugins_list: listed },
      {
        command: 'games_plugins_list',
        error: { code: 'core_failed', message: "Agora could not read this instance's plugin list." },
      },
    );

    await expect(page.getByRole('alert')).toHaveText("Agora could not read this instance's plugin list.");
    await expect(page.getByRole('button', { name: 'Try again' })).toBeVisible();
  });

  test('a reply of the wrong shape is a failure, never an empty list', async ({ page }) => {
    await openPage(page, 'plugins', { games_plugins_list: null });

    await expect(page.getByRole('alert')).toHaveText('Agora could not finish that. Try again.');
    await expect(page.getByTestId('plugin-row')).toHaveCount(0);
  });
});

test.describe('INI & saves tab', () => {
  const skyrimIni = {
    instancePath: 'Skyrim.ini',
    copyExists: true,
    gameFile: 'C:/Users/player/Documents/My Games/Skyrim Special Edition/Skyrim.ini',
    gameFileExists: true,
  };
  const general = { section: 'General', key: 'sLanguage', value: 'ENGLISH' };

  test('edits a key and shows the file as it is after the save', async ({ page }) => {
    const saved = { section: 'General', key: 'sLanguage', value: 'GERMAN' };
    await openPage(page, 'ini-saves', {
      games_ini_get: {
        files: [skyrimIni],
        contents: { instancePath: 'Skyrim.ini', source: 'copy', entries: [general] },
      },
      games_ini_set: {
        changed: true,
        view: {
          files: [skyrimIni],
          contents: { instancePath: 'Skyrim.ini', source: 'copy', entries: [saved] },
        },
      },
      games_saves_get: SHARED_SAVES,
    });

    await expect(page.getByRole('button', { name: /Skyrim\.ini/ })).toHaveAttribute('aria-pressed', 'true');
    const value = page.getByRole('textbox', { name: 'Value of sLanguage in [General]' });
    await expect(value).toHaveValue('ENGLISH');
    await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();

    await value.fill('GERMAN');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.getByRole('status')).toHaveText('Saved sLanguage in [General].');
    await expect(value).toHaveValue('GERMAN');
  });

  test('removing a key asks first, and does nothing when the user cancels', async ({ page }) => {
    await openPage(page, 'ini-saves', {
      games_ini_get: {
        files: [skyrimIni],
        contents: { instancePath: 'Skyrim.ini', source: 'copy', entries: [general] },
      },
      games_ini_set: {
        changed: true,
        view: {
          files: [skyrimIni],
          contents: { instancePath: 'Skyrim.ini', source: 'copy', entries: [] },
        },
      },
      games_saves_get: SHARED_SAVES,
    });

    await page.getByRole('button', { name: 'Remove' }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toContainText('Remove sLanguage from [General]?');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(page.getByRole('status')).toHaveCount(0);
    await expect(page.getByRole('textbox', { name: 'Value of sLanguage in [General]' })).toHaveValue('ENGLISH');

    await page.getByRole('button', { name: 'Remove' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Remove key' }).click();
    await expect(page.getByRole('status')).toHaveText('Removed sLanguage from [General].');
  });

  test('a game with no INI files says there is nothing to edit', async ({ page }) => {
    await openPage(page, 'ini-saves', {
      games_ini_get: { files: [], contents: null },
      games_saves_get: SHARED_SAVES,
    });

    await expect(page.getByText("This game keeps no per-user INI files for this instance's store")).toBeVisible();
  });

  test('switching to own saves says the saves were not moved', async ({ page }) => {
    const sharedSaves = {
      instanceId: INSTANCE_ID,
      choice: 'shared',
      inUse: { path: 'C:/Shared', exists: true, saves: 3, newest: '2026-10-01T10:00:00Z' },
      other: { path: 'C:/Own', exists: false, saves: 0, newest: null },
      settingFile: 'Skyrim.ini',
      setting: 'General/sSaveFolder',
      settingValue: null,
      settingSource: 'nothing',
    };
    await openPage(page, 'ini-saves', {
      games_ini_get: { files: [], contents: null },
      games_saves_get: sharedSaves,
      games_saves_set: {
        previous: 'shared',
        setting: { action: 'set', value: 'C:/Own' },
        saves: { ...sharedSaves, choice: 'own', inUse: { path: 'C:/Own', exists: true, saves: 0, newest: null }, other: sharedSaves.inUse },
      },
    });

    await page.getByRole('button', { name: 'Use own saves' }).click();
    const notice = page.getByRole('status');
    await expect(notice).toContainText('This instance now keeps own saves.');
    await expect(notice).toContainText('The game\'s save setting now names C:/Own.');
    await expect(notice).toContainText('Your saves were not moved, copied or deleted.');
  });

  test('a save setting that was changed by hand is reported, not overwritten', async ({ page }) => {
    const sharedSaves = {
      instanceId: INSTANCE_ID,
      choice: 'own',
      inUse: { path: 'C:/Own', exists: true, saves: 0, newest: null },
      other: { path: 'C:/Shared', exists: true, saves: 0, newest: null },
      settingFile: 'Skyrim.ini',
      setting: 'General/sSaveFolder',
      settingValue: 'C:/Own',
      settingSource: 'copy',
    };
    await openPage(page, 'ini-saves', {
      games_ini_get: { files: [], contents: null },
      games_saves_get: sharedSaves,
      games_saves_set: {
        previous: 'own',
        setting: { action: 'left_alone', value: 'C:/Elsewhere' },
        saves: { ...sharedSaves, choice: 'shared' },
      },
    });

    await page.getByRole('button', { name: 'Use shared saves' }).click();
    await expect(page.getByRole('status')).toContainText('Check it by hand.');
  });

  test('a failed read of the save choice shows its message and leaves the INI section usable', async ({ page }) => {
    await openPage(
      page,
      'ini-saves',
      { games_ini_get: { files: [], contents: null }, games_saves_get: null },
      {
        command: 'games_saves_get',
        error: { code: 'core_failed', message: 'Agora cannot tell which store this instance runs under.' },
      },
    );

    await expect(page.getByRole('alert')).toHaveText('Agora cannot tell which store this instance runs under.');
    await expect(page.getByText("This game keeps no per-user INI files")).toBeVisible();
  });
});
