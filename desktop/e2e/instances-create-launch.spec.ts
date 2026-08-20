import { test, expect, type Page } from '@playwright/test';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Dispatch a synthetic game-exited event through the registered Tauri event
 * listener so the process controller transitions from running → exited.
 */
async function dispatchGameExited(page: Page, instanceId: string) {
  await page.evaluate(
    ({ instanceId }) => {
      const listeners = (window as any).__tauriEventListeners as Map<string, number>;
      const callbacks = (window as any).__callbacks as Map<number, (...args: unknown[]) => void>;
      const handlerId = listeners.get('game-exited');
      if (handlerId != null && callbacks) {
        const cb = callbacks.get(handlerId);
        if (cb) cb({ payload: { instance_id: instanceId, exit_code: 0, outcome: 'success', snapshot_id: null } });
      }
    },
    { instanceId },
  );
}

// ---------------------------------------------------------------------------
// Mock — Instances creation + health-gated launch + exit
// ---------------------------------------------------------------------------

interface CreateLaunchMockOptions {
  direct: boolean;
  healthBlockers?: boolean;
  failLaunch?: boolean;
}

/**
 * Installs a comprehensive Tauri mock that supports:
 *  - Initial navigation / settings
 *  - Instance creation (list_manifest_loaders, list_manifest_mc_versions,
 *    list_loader_versions)
 *  - Stateful list_instances (empty → after creation returns the new instance)
 *  - Health check + launch (direct or delegated)
 *  - Event listeners (game-exited, game-log-batch)
 */
async function installCreateLaunchMock(page: Page, options: CreateLaunchMockOptions) {
  await page.addInitScript(({ options }) => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const commandCalls: Record<string, number> = {};
    const lastCommandArgs: Record<string, Record<string, unknown>> = {};
    const eventListeners = new Map<string, number>();
    let createdInstance: Record<string, unknown> | null = null;

    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        commandCalls[command] = (commandCalls[command] ?? 0) + 1;
        lastCommandArgs[command] = args;

        // ---- Settings ----
        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'launch_mode') return Promise.resolve(options.direct ? 'direct' : 'delegation');
          if (key === 'modrinth_enabled') return Promise.resolve(true);
          if (key === 'last_home_visit') return Promise.resolve(null);
          return Promise.resolve(false);
        }
        if (command === 'set_setting') return Promise.resolve(null);

        // ---- Registry / browse (required by sidebar / ambient calls) ----
        if (command === 'get_registry_status') {
          return Promise.resolve({
            has_cached_db: true,
            cached_tag: 'test',
            cached_schema_version: 5,
            latest_tag: 'test',
            update_available: false,
            checked: true,
            message: 'Registry ready.',
          });
        }
        if (command === 'check_registry_update') return Promise.resolve(null);
        if (command === 'list_categories') return Promise.resolve([]);
        if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
        if (command === 'for_you_items') return Promise.resolve([]);
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'get_lkg_marker') return Promise.resolve(null);

        // ---- Manifest data for the create-instance dialog ----
        if (command === 'list_manifest_loaders') {
          return Promise.resolve(['fabric', 'forge', 'quilt']);
        }
        if (command === 'list_manifest_mc_versions') {
          // Called with or without a loader filter
          if (args.loader === 'forge') return Promise.resolve(['1.20.1']);
          return Promise.resolve(['1.21', '1.20.1']);
        }
        if (command === 'list_loader_versions') {
          const loader = args.loader as string;
          const mcVersion = args.mcVersion as string;
          if (loader === 'fabric' && mcVersion === '1.21') {
            // Deliberately oldest-first: the create dialog must sort newest-first
            // and default to 0.16.9 rather than the first (oldest) entry.
            return Promise.resolve([
              { loader: 'fabric', mc_version: '1.21', loader_version: '0.15.11', file_type: 'stable' },
              { loader: 'fabric', mc_version: '1.21', loader_version: '0.16.9', file_type: 'stable' },
            ]);
          }
          if (loader === 'forge' && mcVersion === '1.20.1') {
            return Promise.resolve([
              { loader: 'forge', mc_version: '1.20.1', loader_version: '50.1.0', file_type: 'recommended' },
            ]);
          }
          return Promise.resolve([
            { loader, mc_version: mcVersion, loader_version: '1.0.0', file_type: 'stable' },
          ]);
        }

        // ---- Instances ----
        if (command === 'list_instances') {
          if (createdInstance) return Promise.resolve([createdInstance]);
          return Promise.resolve([]);
        }
        if (command === 'create_instance') {
          const req = args.request as Record<string, unknown>;
          createdInstance = {
            instance_id: req.instance_id,
            name: req.name,
            minecraft_version: req.minecraft_version,
            loader: req.loader,
            loader_version: req.loader_version,
            is_locked: false,
            last_launched_at: null,
          };
          return Promise.resolve(createdInstance);
        }
        if (command === 'delete_instance') return Promise.resolve(null);
        if (command === 'check_instance_crash') return Promise.resolve(null);
        if (command === 'check_instance_updates') return Promise.resolve([]);

        // ---- Health check ----
        if (command === 'check_instance_health') {
          return Promise.resolve({
            score: options.healthBlockers ? 'red' : 'green',
            blockers: options.healthBlockers
              ? [{ kind: 'incompatible_mod', mod_id: 'blocker-mod', filename: 'blocker.jar', message: 'This mod is incompatible', suggested_action: null }]
              : [],
            warnings: [],
          });
        }

        // ---- Launch ----
        if (command === 'launch_instance_direct') {
          if (options.failLaunch) return Promise.reject(new Error('Launch failed'));
          return Promise.resolve(4242);
        }
        if (command === 'launch_instance') {
          if (options.failLaunch) return Promise.reject(new Error('Launch failed'));
          return Promise.resolve(null);
        }
        if (command === 'kill_process') return Promise.resolve(null);
        if (command === 'query_launch_state') return Promise.resolve(null);

        // ---- Events ----
        if (command === 'plugin:event|listen') {
          eventListeners.set(args.event as string, args.handler as number);
          return Promise.resolve(1);
        }
        if (command === 'plugin:event|unlisten') return Promise.resolve(1);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);

        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __commandCalls: commandCalls,
      __lastCommandArgs: lastCommandArgs,
      __tauriEventListeners: eventListeners,
      __callbacks: callbacks,
    });
  }, { options });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe('Instances — create, launch, and exit flow', () => {

  test('create a new instance, launch it via health-gated controller, and verify running→exited transition', async ({ page }) => {
    await installCreateLaunchMock(page, { direct: true, healthBlockers: false, failLaunch: false });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();

    // ---- Empty state ----
    await expect(page.getByText('No instances yet.')).toBeVisible();

    // ---- Open the create dialog ----
    await page.getByRole('button', { name: 'Create Instance' }).click();
    await expect(page.getByRole('dialog')).toBeVisible();
    await expect(page.getByText('Create Custom Instance')).toBeVisible();

    // ---- Fill the form ----
    const nameInput = page.getByPlaceholder('Optimized Survival');
    await nameInput.fill('My Test Pack');

    // MC version defaults to "1.21" (first from mock)
    // Loader defaults to "fabric" (first from mock)
    // Loader version defaults to "0.16.9 (stable)" — the latest, not the first
    // (oldest) entry the mock returns.
    const loaderVersionSelect = page.locator('label', { hasText: 'Loader version' }).locator('select');
    await expect(loaderVersionSelect).toHaveValue('0.16.9');

    // ---- Submit creation ----
    await page.getByRole('button', { name: 'Create' }).click();

    // Wait for the dialog to close and the instance list to refresh
    await expect(page.getByRole('dialog')).toHaveCount(0);

    // ---- Verify the new instance appears in the list ----
    await expect(page.getByText('My Test Pack')).toBeVisible();
    // Loader and MC version are separate chips on the card.
    await expect(page.getByText('fabric 0.16.9', { exact: true })).toBeVisible();
    await expect(page.getByText('MC 1.21', { exact: true })).toBeVisible();
    await expect(page.getByText('Never launched')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Launch' })).toBeVisible();

    // Verify that create_instance was called
    const createCalls = await page.evaluate(() => {
      const c = (window as any).__commandCalls as Record<string, number>;
      return c['create_instance'] ?? 0;
    });
    expect(createCalls).toBe(1);

    // Verify the instance_id was derived from the name
    const createArgs = await page.evaluate(() => {
      const a = (window as any).__lastCommandArgs as Record<string, Record<string, unknown>>;
      return a['create_instance']?.request as Record<string, unknown> ?? null;
    });
    expect(createArgs?.instance_id).toBe('my-test-pack');
    expect(createArgs?.name).toBe('My Test Pack');

    // ---- Launch the instance ----
    await page.getByRole('button', { name: 'Launch' }).click();

    // Health is all-clear (green) so launch proceeds directly to Running
    await expect(page.getByText(/Running \(PID 4242\)/)).toBeVisible();

    // Verify health check was called once
    const healthCalls = await page.evaluate(() => {
      const c = (window as any).__commandCalls as Record<string, number>;
      return c['check_instance_health'] ?? 0;
    });
    expect(healthCalls).toBe(1);

    // Verify launch_instance_direct was called once
    const directCalls = await page.evaluate(() => {
      const c = (window as any).__commandCalls as Record<string, number>;
      return c['launch_instance_direct'] ?? 0;
    });
    expect(directCalls).toBe(1);

    // ---- Dispatch game-exited event ----
    await dispatchGameExited(page, 'my-test-pack');

    // Running indicator should clear
    await expect(page.getByText(/Running \(PID/)).toHaveCount(0);

    // The card reverts to "Never launched" (our mock never sets last_launched_at)
    await expect(page.getByText('Never launched')).toBeVisible();

    // The Launch button should be available again (process exited state is terminal,
    // so the next click starts fresh).
    await expect(page.getByRole('button', { name: 'Launch' })).toBeVisible();
  });

});
