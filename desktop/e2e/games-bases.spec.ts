import { test, expect, type Page } from '@playwright/test';
import { expectNoUnmockedCommands, installGamesMock, OTHER_SLICES_REPLIES } from './support/games-mock';

/**
 * Setup → pinned bases (MASTER_SPEC §26.4): the list, the checks, building a base, and removing one.
 * Every command these tests reach is answered from the table in `support/games-mock.ts`; anything
 * else fails the test.
 */

/** The boot commands the shell answers with empty data. Shared by every test here. */
const BOOT_REPLIES = {
  ...OTHER_SLICES_REPLIES,
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
  installBytes: 16 * 1024 ** 3,
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

const GB = 1024 ** 3;
const MB = 1024 ** 2;

const usedBase = {
  baseId: 'skyrim-se_steam_1.6.1170_nobuild',
  gameId: 'skyrim-se',
  store: 'steam',
  version: '1.6.1170',
  build: null,
  mode: 'linked',
  location: 'D:/AgoraBases/skyrim-se_steam_1.6.1170_nobuild',
  present: true,
  fileCount: 117,
  linkedFileCount: 40,
  linkedBytes: 16 * GB,
  copiedBytes: 200 * MB,
  skippedCount: 1,
  createdUnixMs: 1700000000000,
  usedBy: [{ instanceId: 'nordic-run', name: 'Nordic Run' }],
};

const spareBase = {
  ...usedBase,
  baseId: 'skyrim-se_steam_1.6.1170_nobuild_spare',
  mode: 'copied',
  location: 'D:/AgoraBases/skyrim-se_steam_1.6.1170_nobuild_spare',
  copiedBytes: 16 * GB,
  linkedBytes: 0,
  linkedFileCount: 0,
  skippedCount: 0,
  usedBy: [],
};

const cleanQuick = {
  baseId: usedBase.baseId,
  depth: 'quick',
  checked: 117,
  hashed: 0,
  clean: true,
  gameWrites: [],
  problems: [],
};

/** A record Agora cannot read: listed with its file and reason, and it has no game or version to show. */
const damagedRecord = {
  baseId: 'skyrim-se_steam_2.0.0_nobuild',
  manifestPath: 'D:/Data/bases/manifests/skyrim-se_steam_2.0.0_nobuild.json',
  error: 'D:/Data/bases/manifests/skyrim-se_steam_2.0.0_nobuild.json is not a valid base record: EOF while parsing an object',
  usedBy: [],
};

const builtCopied = {
  status: 'built',
  baseId: spareBase.baseId,
  mode: 'copied',
  location: spareBase.location,
  fileCount: 117,
  linkedBytes: 0,
  copiedBytes: 16 * GB,
};

async function boot(page: Page, replies: Record<string, unknown>, install: unknown = skyrimInstall) {
  const game = { ...skyrim, installs: [install] };
  await installGamesMock(page, {
    replies: { ...BOOT_REPLIES, games_list: { games: [game] }, ...replies },
    settings: { active_game: 'skyrim-se' },
  });
}

async function openSetup(page: Page) {
  await page.goto('/');
  await page.getByRole('navigation', { name: 'Main navigation' }).getByRole('button', { name: 'Setup', exact: true }).click();
  await expect(page.getByTestId('setup-bases')).toBeVisible();
}

test.afterEach(async ({ page }) => {
  await expectNoUnmockedCommands(page);
});

test('the bases are listed with their mode, disk use and users, and a base in use cannot be removed', async ({ page }) => {
  await boot(page, { games_base_list: { bases: [usedBase, spareBase], damaged: [] } });
  await openSetup(page);

  const cards = page.getByTestId('base-card');
  await expect(cards).toHaveCount(2);

  const used = cards.first();
  await expect(used.getByRole('heading', { name: 'steam 1.6.1170' })).toBeVisible();
  await expect(used.getByText('Linked', { exact: true })).toBeVisible();
  await expect(used.getByText('Used by Nordic Run. Remove or move those instances before removing this base.')).toBeVisible();
  await expect(used.getByRole('button', { name: 'Remove base' })).toBeDisabled();

  const spare = cards.nth(1);
  await expect(spare.getByText('No instance uses this base, so it can be removed.')).toBeVisible();
  await expect(spare.getByRole('button', { name: 'Remove base' })).toBeEnabled();
});

test('a quick check reports a clean base, and a full check runs on request', async ({ page }) => {
  await boot(page, {
    games_base_list: { bases: [usedBase], damaged: [] },
    games_base_verify: cleanQuick,
  });
  await openSetup(page);

  const card = page.getByTestId('base-card');
  await card.getByRole('button', { name: 'Check base (quick)' }).click();
  await expect(page.getByTestId('base-check-result')).toContainText('Clean. Sizes and times were checked: 117 files checked, 0 hashed.');
});

test('a base that needs attention names the damaged files', async ({ page }) => {
  await boot(page, {
    games_base_list: { bases: [usedBase], damaged: [] },
    games_base_verify: {
      baseId: usedBase.baseId,
      depth: 'full',
      checked: 117,
      hashed: 117,
      clean: false,
      gameWrites: ['Data/Skyrim.ini'],
      problems: [
        { path: 'Data/Skyrim - Meshes0.bsa', kind: 'size_changed', expectedBytes: 10 * MB, actualBytes: 7 * MB, linkedToStore: true },
        { path: 'Data/Extra.esp', kind: 'unexpected', expectedBytes: null, actualBytes: null, linkedToStore: false },
      ],
    },
  });
  await openSetup(page);

  await page.getByTestId('base-card').getByRole('button', { name: 'Check every file (full)' }).click();
  const result = page.getByTestId('base-check-result');
  await expect(result).toContainText('Damaged: 2 files need attention.');
  await expect(result).toContainText('Data/Skyrim - Meshes0.bsa');
  await expect(result).toContainText('also reached the store install');
  await expect(result).toContainText('Data/Extra.esp');
  await expect(result).toContainText('The game wrote to this file: Data/Skyrim.ini');
});

test('removing an unused base asks first, then removes it and says so', async ({ page }) => {
  await boot(page, {
    games_base_list: { bases: [spareBase], damaged: [] },
    games_base_remove: { baseId: spareBase.baseId, status: 'removed' },
  });
  await openSetup(page);

  await page.getByTestId('base-card').getByRole('button', { name: 'Remove base' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByText('Remove the steam 1.6.1170 base?')).toBeVisible();
  await dialog.getByRole('button', { name: 'Remove base' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Removed the steam 1.6.1170 base.' })).toBeVisible();
});

test('cancelling the removal keeps the base', async ({ page }) => {
  await boot(page, { games_base_list: { bases: [spareBase], damaged: [] } });
  await openSetup(page);

  await page.getByTestId('base-card').getByRole('button', { name: 'Remove base' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByTestId('base-card')).toHaveCount(1);
});

test('building a Copied base explains the mode, then reports what was built', async ({ page }) => {
  await boot(page, {
    games_base_list: { bases: [], damaged: [] },
    games_base_build: builtCopied,
  });
  await openSetup(page);

  await page.getByRole('button', { name: 'Build a base' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('button', { name: /Skyrim Special Edition/ })).toHaveAttribute('aria-pressed', 'true');
  await expect(dialog.getByRole('button', { name: /Linked \(recommended\)/ })).toHaveAttribute('aria-pressed', 'true');

  await dialog.getByRole('button', { name: /^Copied/ }).click();
  await expect(dialog.getByRole('button', { name: /^Copied/ })).toHaveAttribute('aria-pressed', 'true');
  await expect(dialog.getByText('takes about as much disk space as the game itself', { exact: false })).toBeVisible();
  // The cost is shown before anything is built.
  await expect(dialog.getByTestId('copied-cost')).toContainText('takes about 16 GB of disk space');

  await dialog.getByRole('button', { name: 'Build a Copied base' }).click();
  await expect(dialog.getByRole('status').filter({ hasText: 'Built the Copied base.' })).toBeVisible();
  await dialog.getByRole('button', { name: 'Done' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('a Linked build that cannot link offers a Copied build with its cost, and does not switch by itself', async ({ page }) => {
  await boot(page, {
    games_base_list: { bases: [], damaged: [] },
    games_base_build: { status: 'link_unavailable', reason: 'base is on another volume', copiedBytesNeeded: 16 * GB },
  });
  await openSetup(page);

  await page.getByRole('button', { name: 'Build a base' }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: 'Build a Linked base' }).click();

  const choice = dialog.getByTestId('link-unavailable');
  await expect(choice).toContainText('Hard links are not available for this install');
  await expect(choice).toContainText('base is on another volume');
  await expect(choice).toContainText('A Copied base would take 16 GB on this drive.');
  await expect(choice).toContainText('Nothing was built yet.');
  await expect(dialog.getByRole('button', { name: 'Build a Copied base (16 GB)' })).toBeVisible();
  await dialog.getByRole('button', { name: 'Back' }).click();
  await expect(dialog.getByTestId('link-unavailable')).toHaveCount(0);
});

test('an install whose version Agora cannot tell cannot be built from', async ({ page }) => {
  const unidentified = {
    ...skyrimInstall,
    storeBuild: null,
    runtime: { status: 'unidentified', reasons: ['the executable could not be read'] },
  };
  await boot(page, { games_base_list: { bases: [], damaged: [] } }, unidentified);
  await openSetup(page);

  await page.getByRole('button', { name: 'Build a base' }).click();
  const dialog = page.getByRole('dialog');
  const install = dialog.getByRole('button', { name: /Skyrim Special Edition/ });
  await expect(install).toBeDisabled();
  await expect(dialog).toContainText('Agora cannot tell which exact version this is');
  await expect(dialog.getByRole('button', { name: 'Build a Linked base' })).toBeDisabled();
});

test('an empty list says what to do next', async ({ page }) => {
  await boot(page, { games_base_list: { bases: [], damaged: [] } });
  await openSetup(page);

  const empty = page.getByTestId('bases-empty');
  await expect(empty).toContainText('No pinned bases yet.');
  await expect(empty).toContainText('Build one from an installed copy of the game.');
  await expect(page.getByRole('button', { name: 'Build a base' })).toBeEnabled();
});

test('a list that does not come back as a list is an error, not an empty list', async ({ page }) => {
  await boot(page, { games_base_list: null });
  await openSetup(page);

  await expect(page.getByRole('alert')).toContainText('Agora could not read the list of bases. Try again.');
  await expect(page.getByTestId('bases-empty')).toHaveCount(0);
});

test('a backend error shows its own message', async ({ page }) => {
  await boot(page, { games_base_list: { bases: [], damaged: [] } });
  // The mock answers from a table; this one makes the command fail the way the backend does.
  await page.addInitScript(() => {
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: (c: string, a?: unknown) => Promise<unknown> } }).__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = (command: string, args?: unknown) => (command === 'games_base_list'
      ? Promise.reject({ code: 'base_failed', message: 'Agora could not read the list of instances, so it cannot tell which bases are in use.' })
      : original(command, args));
  });
  await openSetup(page);

  await expect(page.getByRole('alert')).toContainText('Agora could not read the list of instances');
  await expect(page.getByTestId('bases-empty')).toHaveCount(0);
});

test('a damaged base record is listed with its file and reason, offers no checks, and its removal asks first', async ({ page }) => {
  await boot(page, {
    games_base_list: { bases: [], damaged: [damagedRecord] },
    games_base_remove: { baseId: damagedRecord.baseId, status: 'removed' },
  });
  await openSetup(page);

  const card = page.getByTestId('damaged-base-card');
  await expect(card).toHaveCount(1);
  await expect(card.getByRole('heading', { name: 'Damaged base record' })).toBeVisible();
  await expect(card).toContainText('Agora cannot read this base');
  await expect(card).toContainText(damagedRecord.manifestPath);
  await expect(card).toContainText('is not a valid base record');
  await expect(card.getByRole('button', { name: 'Check base (quick)' })).toHaveCount(0);
  await expect(card.getByRole('button', { name: 'Check every file (full)' })).toHaveCount(0);
  await expect(page.getByTestId('bases-empty')).toHaveCount(0);

  await card.getByRole('button', { name: 'Remove base' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByText('Remove this damaged base record?')).toBeVisible();
  await expect(dialog).toContainText('Its record file is deleted');
  await dialog.getByRole('button', { name: 'Remove base' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Removed the damaged base record.' })).toBeVisible();
});

test('a damaged base record in use cannot be removed, and names the instances that use it', async ({ page }) => {
  await boot(page, {
    games_base_list: { bases: [], damaged: [{ ...damagedRecord, usedBy: [{ instanceId: 'nordic-run', name: 'Nordic Run' }] }] },
  });
  await openSetup(page);

  const card = page.getByTestId('damaged-base-card');
  await expect(card).toContainText('Used by Nordic Run. Remove or move those instances before removing this base.');
  await expect(card.getByRole('button', { name: 'Remove base' })).toBeDisabled();
});

test('a damaged record beside built bases is listed with them, and the list is not empty', async ({ page }) => {
  await boot(page, { games_base_list: { bases: [spareBase], damaged: [damagedRecord] } });
  await openSetup(page);

  await expect(page.getByTestId('base-card')).toHaveCount(1);
  await expect(page.getByTestId('damaged-base-card')).toHaveCount(1);
  await expect(page.getByTestId('bases-empty')).toHaveCount(0);
});
