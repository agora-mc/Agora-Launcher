import { test, expect } from '@playwright/test';

/**
 * The app with plugins switched **on**.
 *
 * Everything else in this suite runs with them off, which is the default and
 * the important case — but it means a plugin surface that crashed the app
 * would not fail a single test. These run the real React tree with a plugin
 * contributing a page and offering to render the home screen, over a mocked
 * Tauri bridge.
 *
 * The mock ends in `Promise.resolve(null)` like every other spec here, which
 * is deliberate: a plugin command this file forgets to name must still not
 * take the window down. That is the failure shape these components were
 * hardened against.
 */

const contribution = {
  id: 'acme.probe/overview',
  pluginId: 'acme.probe',
  localId: 'overview',
  kind: 'page',
  title: 'Probe overview',
};

const offer = {
  id: 'acme.probe/home',
  pluginId: 'acme.probe',
  localId: 'home',
  pluginName: 'Probe',
  title: 'Probe home',
  description: 'A different first screen.',
  surface: 'home',
  export: 'home',
};

const plugin = {
  id: 'acme.probe',
  name: 'Probe',
  version: '1.0.0',
  license: 'MIT',
  enabled: true,
  running: true,
  development: true,
  status: { state: 'ready' },
  statusText: 'Ready',
  capabilities: ['instance:read'],
  declaredHosts: [],
  contributions: [contribution],
  droppedEvents: 0,
  updateSource: null,
  lastUpdateCheck: null,
  restorableData: null,
  definitions: {
    pages: [{ id: 'overview', title: 'Probe overview', view: { kind: 'host', export: 'overview' } }],
    replacements: [
      { id: 'home', title: 'Probe home', surface: 'home', view: { kind: 'host', export: 'home' } },
    ],
    instancePanels: [],
    commands: [],
    settings: [],
    diagnostics: [],
    launchChecks: [],
  },
};

/** `selectedSurface` decides whether the plugin or Agora draws the home page. */
async function boot(page: import('@playwright/test').Page, selectedSurface: string | null) {
  await page.addInitScript(
    ({ plugin, contribution, offer, selectedSurface }) => {
      let selected = selectedSurface;
      Object.assign(window, {
        __TAURI_INTERNALS__: {
          transformCallback() { return 1; },
          unregisterCallback() {},
          invoke(command: string, args: Record<string, unknown> = {}) {
            if (command === 'get_setting') {
              if (args.key === 'onboarding_complete') return Promise.resolve(true);
              if (args.key === 'plugins_enabled') return Promise.resolve(true);
              return Promise.resolve(null);
            }
            if (command === 'set_setting') return Promise.resolve(null);
            if (command === 'plugins_enabled') return Promise.resolve(true);
            if (command === 'start_plugins') return Promise.resolve([]);
            if (command === 'list_plugins') return Promise.resolve([plugin]);
            if (command === 'list_plugin_contributions') return Promise.resolve([contribution]);
            if (command === 'plugin_surfaces') {
              const effective = selected === offer.id ? offer : null;
              return Promise.resolve([
                {
                  surface: 'home',
                  title: 'Home',
                  offers: [offer],
                  selected,
                  effective,
                  fallbackReason: null,
                },
              ]);
            }
            if (command === 'plugin_set_surface') {
              selected = (args.contributionId as string | null) ?? null;
              return Promise.resolve(null);
            }
            if (command === 'render_plugin_view') {
              return Promise.resolve({
                title: args.export === 'home' ? 'Drawn by the plugin' : 'Probe overview body',
                blocks: [
                  { type: 'stats', items: [{ label: 'Export', value: String(args.export) }] },
                ],
              });
            }
            if (command.startsWith('plugin:event|')) return Promise.resolve(1);
            if (command.startsWith('list_')) return Promise.resolve([]);
            // Everything unnamed resolves null on purpose. See the file docs.
            return Promise.resolve(null);
          },
        },
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      });
    },
    { plugin, contribution, offer, selectedSurface },
  );
  await page.goto('/');
}

test('the app renders with plugins enabled and a plugin contributing a page', async ({ page }) => {
  await boot(page, null);

  // The shell is the thing at risk: a plugin surface that throws during render
  // takes the sidebar with it, and that is how this class of bug has shown up
  // before.
  await expect(page.getByRole('button', { name: 'Home' }).first()).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText('Probe overview').first()).toBeVisible();
});

test('an offered home replacement does not take the screen until it is chosen', async ({ page }) => {
  await boot(page, null);
  await expect(page.getByRole('button', { name: 'Home' }).first()).toBeVisible({ timeout: 15_000 });

  // The offer exists and the plugin is running, and Agora still draws home.
  await expect(page.getByText('Drawn by the plugin')).toHaveCount(0);
});

test('the chosen replacement is what draws the home screen', async ({ page }) => {
  await boot(page, 'acme.probe/home');
  await expect(page.getByText('Drawn by the plugin')).toBeVisible({ timeout: 15_000 });
});

test('an unmocked plugin command does not take the window down', async ({ page }) => {
  const crashes: string[] = [];
  page.on('pageerror', (error) => crashes.push(error.message));

  // `plugin_surfaces` is answered; everything else a future build might call
  // is not, and resolves null through the fallback.
  await boot(page, null);
  await expect(page.getByRole('button', { name: 'Home' }).first()).toBeVisible({ timeout: 15_000 });
  expect(crashes, `page errors: ${crashes.join(' | ')}`).toHaveLength(0);
});
