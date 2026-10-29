import { test, expect, type Page } from '@playwright/test';
import { expectNoUnmockedCommands, installGamesMock } from './support/games-mock';

/**
 * The Tools page of an instance of a non-Minecraft game (MASTER_SPEC §26.9): the declared tools, their
 * output, runs with the capture choice, the fallbacks that are asked about, and the swap banner. Every
 * command these tests reach is answered from the table in `support/games-mock.ts`; anything else fails.
 *
 * The page is opened directly: the instances slice owns the way in, so the test writes the history
 * entry the app restores on load (the same entry a reload restores).
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

const skyrim = {
  gameId: 'skyrim-se',
  name: 'Skyrim Special Edition',
  installs: [skyrimInstall],
};

const INSTANCE_ID = 'sk-1';

const nemesisRow = {
  toolId: 'nemesis',
  name: 'Nemesis',
  declared: true,
  current: '2',
  previous: '1',
  status: 'current',
  conflicts: true,
};
const pandoraRow = {
  toolId: 'pandora',
  name: 'Pandora',
  declared: true,
  current: '1',
  previous: null,
  status: 'stale',
  conflicts: true,
};
const bodyslideRow = {
  toolId: 'bodyslide',
  name: 'BodySlide',
  declared: true,
  current: null,
  previous: null,
  status: null,
  conflicts: false,
};

const toolsView = { instanceId: INSTANCE_ID, tools: [nemesisRow, pandoraRow, bodyslideRow] };

const swapCosts = {
  installDir: 'D:/Games/Skyrim Special Edition',
  dataFiles: 368,
  dataBytes: 5 * 1024 * 1024 * 1024 + 512 * 1024 * 1024,
  sameVolume: true,
  copyKind: 'kept',
  copyFolder: 'D:/Agora/swap',
  keptCopyExists: false,
  copyFiles: 368,
  copyBytes: 5 * 1024 * 1024 * 1024 + 512 * 1024 * 1024,
  freeBytes: 120 * 1024 * 1024 * 1024,
  steamRunning: false,
};

const outcome = {
  toolId: 'nemesis',
  name: 'Nemesis',
  run: '0000000000001-abcd1234',
  capture: 'vfs',
  captureReason: null,
  promoted: true,
  current: '3',
  previous: '2',
  written: ['Data/meshes/actors/character/a.hkx'],
  deleted: [],
  exitCode: 0,
  cancelled: false,
  failedFolder: null,
  failure: null,
  copiedLayer: null,
  installChanges: { created: [], changed: [], deleted: [], mayBeIncomplete: false, unavailable: null },
  swap: null,
  offers: [],
  unchangedDropped: 0,
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

const NO_INTERRUPTED_SWAP = { interrupted: null };

interface ToolsTable {
  list?: unknown;
  run?: unknown;
  swapCosts?: unknown;
  swapStatus?: unknown;
  diff?: unknown;
  remove?: unknown;
  restore?: unknown;
}

/** Install the games mock with the tools replies, then record every call the page makes to a tools command. */
async function boot(page: Page, tools: ToolsTable = {}) {
  await installGamesMock(page, {
    replies: {
      ...BOOT_REPLIES,
      games_list: { games: [skyrim] },
      games_tools_list: tools.list ?? toolsView,
      games_tools_run: tools.run ?? { status: 'finished', outcome },
      games_tools_swap_costs: tools.swapCosts ?? swapCosts,
      games_tools_swap_status: tools.swapStatus ?? NO_INTERRUPTED_SWAP,
      games_tools_diff: tools.diff ?? {
        toolId: 'nemesis',
        previous: '2',
        current: '3',
        added: ['Data/meshes/actors/character/new.hkx'],
        changed: ['Data/meshes/actors/character/a.hkx'],
        removed: [],
      },
      games_tools_remove: tools.remove ?? { removedGenerations: ['1', '2'], view: { instanceId: INSTANCE_ID, tools: [{ ...nemesisRow, current: null, previous: null, status: null, conflicts: false }, pandoraRow, bodyslideRow] } },
      games_tools_swap_restore: tools.restore ?? { restoredTools: ['bodyslide'] },
      games_tools_rollback: toolsView,
    },
    settings: { active_game: 'skyrim-se' },
  });
  // Record each tools call, so a test can see what the page asked for and when. Installed after the
  // mock, which it wraps; a call it does not answer still reaches the mock and fails there.
  await page.addInitScript(() => {
    const target = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
      __toolCalls?: { command: string; args: Record<string, unknown> }[];
    };
    const inner = target.__TAURI_INTERNALS__.invoke;
    target.__toolCalls = [];
    target.__TAURI_INTERNALS__.invoke = (command: string, args: Record<string, unknown> = {}) => {
      if (command.startsWith('games_tools_')) target.__toolCalls?.push({ command, args });
      return inner(command, args);
    };
  });
}

/** Open the instance's Tools page: write the history entry the app restores, then load it. */
async function openToolsPage(page: Page) {
  await page.goto('/');
  await page.evaluate((instanceId) => {
    window.history.replaceState({
      __agora: { type: 'game-instance', gameId: 'skyrim-se', instanceId, page: 'tools' },
      __agoraDepth: 0,
    }, '');
  }, INSTANCE_ID);
  await page.reload();
  await expect(page.getByTestId('instance-tools')).toBeVisible();
}

async function toolCalls(page: Page): Promise<{ command: string; args: Record<string, unknown> }[]> {
  return page.evaluate(() => (window as unknown as { __toolCalls: { command: string; args: Record<string, unknown> }[] }).__toolCalls);
}

function card(page: Page, name: string) {
  return page.getByRole('article', { name });
}

test.afterEach(async ({ page }) => {
  await expectNoUnmockedCommands(page);
});

test('the declared tools show their output, a stale badge and the conflict warning', async ({ page }) => {
  await boot(page);
  await openToolsPage(page);

  await expect(card(page, 'Nemesis').getByTestId('tool-status')).toHaveText('Up to date');
  await expect(card(page, 'Nemesis')).toContainText('Output: generation 2, with generation 1 kept to roll back to.');
  await expect(card(page, 'Pandora').getByTestId('tool-status')).toHaveText('Out of date');
  await expect(card(page, 'BodySlide').getByTestId('tool-status')).toHaveText('Not built yet');
  await expect(card(page, 'BodySlide')).toContainText('No output here yet.');

  // Both tools with output that conflict are named, once each.
  await expect(page.getByTestId('tool-conflict')).toHaveCount(2);
  await expect(page.getByTestId('tool-conflict').first()).toContainText('Nemesis and Pandora both have output here');
});

test('a run that finishes says it is the next generation, and the page asks nothing more', async ({ page }) => {
  await boot(page);
  await openToolsPage(page);

  await card(page, 'Nemesis').getByRole('button', { name: 'Run Nemesis again' }).click();

  await expect(page.getByTestId('tool-outcome')).toContainText('Nemesis finished, and its output is now generation 3.');
  await expect(page.getByTestId('tool-outcome')).toContainText('Captured by the virtual file system.');
  await expect(page.getByTestId('tool-swap-confirm')).toHaveCount(0);

  const calls = await toolCalls(page);
  expect(calls.filter((call) => call.command === 'games_tools_run')).toEqual([
    { command: 'games_tools_run', args: { instanceId: INSTANCE_ID, toolId: 'nemesis', capture: 'auto', refusal: null, confirmSwap: false } },
  ]);
});

test('a discarded run says why and where its files were kept, and changed nothing', async ({ page }) => {
  await boot(page, {
    run: {
      status: 'finished',
      outcome: { ...outcome, promoted: false, current: '2', previous: '1', failure: 'exit code 3', failedFolder: 'D:/Agora-wt2/failed-1' },
    },
  });
  await openToolsPage(page);

  await card(page, 'Nemesis').getByRole('button', { name: 'Run Nemesis again' }).click();

  const result = page.getByTestId('tool-outcome');
  await expect(result).toContainText('did not produce usable output, so nothing changed.');
  await expect(result).toContainText('Why: exit code 3');
  await expect(result).toContainText('D:/Agora-wt2/failed-1');
});

test('a virtual file system that cannot start asks before any other tier runs', async ({ page }) => {
  await boot(page, {
    run: { status: 'vfsRefused', reason: 'injection failed on this install', next: 'links', offers: [] },
  });
  await openToolsPage(page);

  await card(page, 'Nemesis').getByRole('button', { name: 'Run Nemesis again' }).click();

  const refused = page.getByTestId('tool-refused');
  await expect(refused).toContainText('The game’s file layer could not start, so nothing ran.');
  await expect(refused).toContainText('injection failed on this install');
  // Nothing ran: no output shown, and only the first run was asked for.
  await expect(page.getByTestId('tool-outcome')).toHaveCount(0);
  expect((await toolCalls(page)).filter((call) => call.command === 'games_tools_run')).toHaveLength(1);

  await refused.getByRole('button', { name: 'Run with link capture' }).click();
  await expect.poll(async () => (await toolCalls(page)).filter((call) => call.command === 'games_tools_run').length).toBe(2);
  const second = (await toolCalls(page)).filter((call) => call.command === 'games_tools_run')[1];
  expect(second.args).toEqual({
    instanceId: INSTANCE_ID,
    toolId: 'nemesis',
    capture: 'links',
    refusal: 'injection failed on this install',
    confirmSwap: false,
  });
});

test('a full-copy swap is never run on the first click: its costs are shown and it runs when confirmed', async ({ page }) => {
  await boot(page);
  await openToolsPage(page);

  await card(page, 'Nemesis').getByRole('combobox').selectOption('swap');
  await card(page, 'Nemesis').getByRole('button', { name: 'Run Nemesis again' }).click();

  const confirm = page.getByTestId('tool-swap-confirm');
  await expect(confirm).toContainText('You chose the full-copy swap.');
  await expect(page.getByTestId('tool-swap-costs')).toContainText('Disk space: the copy needs 5.5 GB');
  await expect(page.getByTestId('tool-swap-costs')).toContainText('The game’s real Data folder is renamed');
  expect((await toolCalls(page)).some((call) => call.command === 'games_tools_run')).toBe(false);

  await confirm.getByRole('button', { name: 'Run by full-copy swap' }).click();
  await expect(page.getByTestId('tool-outcome')).toContainText('generation 3');
  const runs = (await toolCalls(page)).filter((call) => call.command === 'games_tools_run');
  expect(runs).toEqual([
    { command: 'games_tools_run', args: { instanceId: INSTANCE_ID, toolId: 'nemesis', capture: 'swap', refusal: null, confirmSwap: true } },
  ]);
});

test('a tool that works on the real install asks for a swap, with its costs', async ({ page }) => {
  await boot(page, { run: { status: 'needsSwap', tool: 'bodyslide' } });
  await openToolsPage(page);

  await card(page, 'BodySlide').getByRole('button', { name: 'Build BodySlide' }).click();

  await expect(page.getByTestId('tool-swap-confirm')).toContainText('This tool works on the game’s real install folder');
  await expect(page.getByTestId('tool-swap-costs')).toBeVisible();
  expect((await toolCalls(page)).filter((call) => call.command === 'games_tools_run')).toHaveLength(1);
});

test('removing a tool’s output asks first, and nothing is removed without the confirmation', async ({ page }) => {
  await boot(page);
  await openToolsPage(page);

  await card(page, 'Nemesis').getByRole('button', { name: 'Remove output' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Remove Nemesis’s output?');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  expect((await toolCalls(page)).some((call) => call.command === 'games_tools_remove')).toBe(false);

  await card(page, 'Nemesis').getByRole('button', { name: 'Remove output' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Remove output' }).click();
  await expect(card(page, 'Nemesis')).toContainText('Removed Nemesis, with 2 generation folder(s).');
  await expect(card(page, 'Nemesis').getByTestId('tool-status')).toHaveText('Not built yet');
});

test('Show changes lists what the latest generation added and changed', async ({ page }) => {
  await boot(page);
  await openToolsPage(page);

  await card(page, 'Nemesis').getByRole('button', { name: 'Show changes' }).click();

  const diff = page.getByTestId('tool-diff');
  await expect(diff).toContainText('Generation 3 compared with generation 2');
  await expect(diff).toContainText('Added (1)');
  await expect(diff).toContainText('Data/meshes/actors/character/new.hkx');
  await expect(diff).toContainText('Changed (1)');
});

test('an interrupted swap is shown at the top, and putting it back says what was restored', async ({ page }) => {
  await boot(page, { swapStatus: { interrupted: { tool: 'bodyslide', running: false, unreadable: null } } });
  await openToolsPage(page);

  const banner = page.getByTestId('tool-swap-recovery');
  await expect(banner).toContainText('A tool swap was interrupted');
  await expect(banner).toContainText('The swap was for bodyslide.');

  await banner.getByRole('button', { name: 'Put the Data folder back' }).click();
  // The mock answers the status the same way after the restore, so the banner stays. What this test
  // owns is the reply the page shows for the restore itself, and the call it made.
  await expect(page.getByRole('status').filter({ hasText: 'The game’s Data folder is back where it belongs (the swap for bodyslide).' })).toBeVisible();
  expect((await toolCalls(page)).filter((call) => call.command === 'games_tools_swap_restore')).toEqual([
    { command: 'games_tools_swap_restore', args: { instanceId: INSTANCE_ID } },
  ]);
});

test('a swap that is still running cannot be put back yet, and the page says so', async ({ page }) => {
  await boot(page, { swapStatus: { interrupted: { tool: 'nemesis', running: true, unreadable: null } } });
  await openToolsPage(page);

  const banner = page.getByTestId('tool-swap-recovery');
  await expect(banner).toContainText('A tool is still running from the game folder.');
  await expect(banner.getByRole('button', { name: 'Put the Data folder back' })).toHaveCount(0);
});

test('a game that declares no tools says so, and says nothing is to be built', async ({ page }) => {
  await boot(page, { list: { instanceId: INSTANCE_ID, tools: [] } });
  await openToolsPage(page);

  await expect(page.getByTestId('tools-empty')).toContainText('This game declares no tools');
  await expect(page.getByTestId('tool-card')).toHaveCount(0);
});

test('a tools list that fails shows the error, with a way to try again', async ({ page }) => {
  await boot(page);
  // The list answers with an error from the core, not with a list: the page must show that, not "no tools".
  // The failure lasts until the test lifts it: React's StrictMode runs the load twice on mount, so a
  // failure that lasted one call would be answered by the second call, and the page would show the list.
  await page.addInitScript(() => {
    const target = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
      __failToolsList?: boolean;
    };
    const inner = target.__TAURI_INTERNALS__.invoke;
    target.__failToolsList = true;
    target.__TAURI_INTERNALS__.invoke = (command: string, args: Record<string, unknown> = {}) => {
      if (command === 'games_tools_list' && target.__failToolsList) {
        return Promise.reject({ code: 'core_failed', message: 'The instance could not be read.' });
      }
      return inner(command, args);
    };
  });
  await openToolsPage(page);

  const alert = page.getByRole('alert').filter({ hasText: 'The tools could not be listed: The instance could not be read.' });
  await expect(alert).toBeVisible();
  await expect(page.getByTestId('tools-empty')).toHaveCount(0);

  await page.evaluate(() => {
    (window as unknown as { __failToolsList?: boolean }).__failToolsList = false;
  });
  await alert.getByRole('button', { name: 'Try again' }).click();
  await expect(card(page, 'Nemesis')).toBeVisible();
});
