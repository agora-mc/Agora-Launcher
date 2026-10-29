import { test, expect, type Page } from '@playwright/test';

// Worlds & Servers tab, Quick Play, and the per-instance window size.

const INSTANCE_DETAIL: Record<string, unknown> = {
  row: {
    instance_id: 'test-instance',
    name: 'Test Instance',
    minecraft_version: '26.2',
    loader: 'fabric',
    loader_version: '0.16.9',
    is_modpack: false,
    is_locked: false,
    last_launched_at: '2026-07-12T10:00:00Z',
    jvm_memory_mb: 4096,
    jvm_gc: 'G1GC',
    jvm_custom_args: '',
    created_at: '2026-06-01T00:00:00Z',
    total_play_ms: 5 * 3_600_000,
  },
  manifest: {
    instance_id: 'test-instance',
    name: 'Test Instance',
    created_from_pack: null,
    minecraft_version: '26.2',
    loader: 'fabric',
    loader_version: '0.16.9',
    is_locked: false,
    mods: [],
    resourcepacks: [],
    shaders: [],
    datapacks: [],
    worlds: [],
    user_preferences: {},
  },
  pack_gaps: [],
};

const WORLDS = [
  {
    folder: 'Alpha',
    name: 'Alpha Base',
    last_played_ms: Date.parse('2026-07-11T10:00:00Z'),
    game_mode: 'survival',
    hardcore: false,
    version_name: '26.2',
    size_bytes: 12_000_000,
    icon_data_url: null,
  },
];

const SERVERS = [{ name: 'Home', address: 'play.example.net', icon_data_url: null }];

async function installWorldsMock(page: Page) {
  await page.addInitScript(
    (params: { detail: Record<string, unknown>; worlds: unknown[]; servers: unknown[] }) => {
      const callbacks = new Map<number, (...args: unknown[]) => void>();
      let callbackId = 0;
      const commandArgs: Record<string, Record<string, unknown>> = {};
      const internals = {
        transformCallback(callback: (...args: unknown[]) => void) {
          const id = ++callbackId;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback(id: number) { callbacks.delete(id); },
        invoke(command: string, args: Record<string, unknown> = {}) {
          commandArgs[command] = args;
          if (command === 'get_setting') {
            const key = args.key as string;
            if (key === 'onboarding_complete') return Promise.resolve(true);
            if (key === 'launch_mode') return Promise.resolve('delegation');
            if (key === 'advanced_mode') return Promise.resolve('true');
            return Promise.resolve(null);
          }
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);
          if (command === 'get_instance_detail') return Promise.resolve(params.detail);
          if (command === 'list_worlds') return Promise.resolve(params.worlds);
          if (command === 'list_servers') return Promise.resolve(params.servers);
          if (command === 'check_instance_health') {
            return Promise.resolve({ blockers: [], warnings: [], recommendations: [], scan_token: 'scan-1' });
          }
          if (command === 'launch_instance_direct') return Promise.resolve(4242);
          if (command === 'ping_server') {
            return Promise.resolve({
              online: true, latency_ms: 42, players_online: 3, players_max: 20,
              version: 'Paper 26.2', motd: 'Welcome home', icon_data_url: null, error: null,
            });
          }
          if (command === 'list_screenshots') {
            return Promise.resolve([{ file: 'castle.png', size_bytes: 10, taken: '2026-07-11T10:00:00Z', path: 'C:/x/castle.png' }]);
          }
          if (command === 'read_screenshot') {
            return Promise.resolve('data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==');
          }
          if (command === 'compute_gc_args') {
            return Promise.resolve({
              profile: 'high_efficiency', jvm_args: '', heap_mb: 4096,
              total_ram_mb: 32768, cpu_threads: 8, recommended: true,
            });
          }
          if (command === 'list_instances') return Promise.resolve([]);
          if (command === 'list_snapshots') return Promise.resolve([]);
          if (command === 'list_categories') return Promise.resolve([]);
          if (command === 'list_manifest_loaders') return Promise.resolve([]);
          if (command === 'list_manifest_mc_versions') return Promise.resolve([]);
          return Promise.resolve(null);
        },
      };
      Object.assign(window as unknown as Record<string, unknown>, {
        __TAURI_INTERNALS__: internals,
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
        __commandArgs: commandArgs,
      });
    },
    { detail: INSTANCE_DETAIL, worlds: WORLDS, servers: SERVERS },
  );
}

const argsOf = (page: Page, command: string) => page.evaluate(
  (name) => (window as unknown as { __commandArgs: Record<string, Record<string, unknown>> }).__commandArgs[name] ?? null,
  command,
);

async function openWorldsTab(page: Page) {
  await page.addInitScript(() => {
    window.history.replaceState({ __agora: { type: 'instance-detail', instanceId: 'test-instance' } }, '');
  });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Test Instance' })).toBeVisible();
  await page.getByRole('button', { name: 'Worlds & Servers' }).click();
  await expect(page.getByRole('heading', { name: 'Worlds', exact: true })).toBeVisible();
}

test('shows play time in the instance header', async ({ page }) => {
  await installWorldsMock(page);
  await openWorldsTab(page);
  await expect(page.getByText('· 5 h played')).toBeVisible();
});

test('lists worlds and servers, and Play opens the world through direct launch', async ({ page }) => {
  await installWorldsMock(page);
  await openWorldsTab(page);

  const world = page.getByTestId('world-row');
  await expect(world.getByText('Alpha Base')).toBeVisible();
  await expect(world.getByText(/Survival · 26\.2/)).toBeVisible();
  await expect(page.getByTestId('server-row').getByText('play.example.net')).toBeVisible();

  await world.getByRole('button', { name: 'Play' }).click();
  await expect.poll(() => argsOf(page, 'launch_instance_direct')).toMatchObject({
    instanceId: 'test-instance',
    quickPlay: { kind: 'singleplayer', target: 'Alpha' },
  });
});

test('Join sends the server address as Quick Play', async ({ page }) => {
  await installWorldsMock(page);
  await openWorldsTab(page);
  await page.getByTestId('server-row').getByRole('button', { name: 'Join' }).click();
  await expect.poll(() => argsOf(page, 'launch_instance_direct')).toMatchObject({
    quickPlay: { kind: 'multiplayer', target: 'play.example.net' },
  });
});

test('Rename asks for a name and keeps the folder', async ({ page }) => {
  await installWorldsMock(page);
  await openWorldsTab(page);
  await page.getByTestId('world-row').getByRole('button', { name: 'Rename' }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('textbox').fill('Second Base');
  await dialog.getByRole('button', { name: 'Rename' }).click();
  await expect.poll(() => argsOf(page, 'rename_world')).toEqual({
    instanceId: 'test-instance',
    folder: 'Alpha',
    name: 'Second Base',
  });
});

test('per-instance window size is saved from Java & Args', async ({ page }) => {
  await installWorldsMock(page);
  await openWorldsTab(page);
  await page.getByRole('button', { name: 'Java & Args' }).click();
  await page.getByRole('radio', { name: 'Fixed size' }).check();
  await page.getByRole('button', { name: '1600×900' }).click();
  await page.getByRole('button', { name: 'Save window size' }).click();
  await expect.poll(() => argsOf(page, 'set_instance_window_size')).toEqual({
    instanceId: 'test-instance',
    width: 1600,
    height: 900,
  });
});

test('servers show their live status', async ({ page }) => {
  await installWorldsMock(page);
  await openWorldsTab(page);
  const server = page.getByTestId('server-row');
  await expect(server.getByText('● Online')).toBeVisible();
  await expect(server.getByText(/3\/20 players · 42 ms · Paper 26\.2/)).toBeVisible();
  await expect(server.getByText('Welcome home')).toBeVisible();
});

test('sharing switches send both choices', async ({ page }) => {
  await installWorldsMock(page);
  await openWorldsTab(page);
  // The mock never reports the change back, so assert what was sent, not the box.
  await page.getByRole('checkbox', { name: /Server list/ }).click();
  await expect.poll(() => argsOf(page, 'set_instance_sharing')).toEqual({
    instanceId: 'test-instance', options: false, servers: true,
  });
});

test('launch commands and environment variables are saved', async ({ page }) => {
  await installWorldsMock(page);
  await openWorldsTab(page);
  await page.getByRole('button', { name: 'Java & Args' }).click();
  await page.getByLabel('Pre-launch command').fill('python sync.py');
  await page.getByLabel('Environment variables').fill('FOO=bar\nEMPTY=');
  await page.getByRole('button', { name: 'Save launch commands' }).click();
  await expect.poll(() => argsOf(page, 'set_instance_launch_hooks')).toEqual({
    instanceId: 'test-instance', preLaunch: 'python sync.py', postExit: '', env: { FOO: 'bar', EMPTY: '' },
  });
});

test('notes are saved from the Notes tab', async ({ page }) => {
  await installWorldsMock(page);
  await openWorldsTab(page);
  await page.getByRole('button', { name: 'Notes', exact: true }).click();
  await page.getByLabel('Instance notes').fill('Seed 1234');
  await page.getByRole('button', { name: 'Save notes' }).click();
  await expect.poll(() => argsOf(page, 'set_instance_notes')).toEqual({ instanceId: 'test-instance', notes: 'Seed 1234' });
});

test('screenshots render and open in a viewer', async ({ page }) => {
  await installWorldsMock(page);
  await openWorldsTab(page);
  await page.getByRole('button', { name: 'Screenshots', exact: true }).click();
  const shot = page.getByTestId('screenshot');
  await expect(shot.getByText('castle.png')).toBeVisible();
  await shot.getByRole('button', { name: 'Open castle.png' }).click();
  const viewer = page.getByRole('dialog', { name: 'castle.png' });
  await expect(viewer.getByRole('button', { name: 'Show in folder' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(viewer).toHaveCount(0);
});
