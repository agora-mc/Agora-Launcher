import { test, expect, type Page } from '@playwright/test';

// ---------------------------------------------------------------------------
// Event dispatch helper
// ---------------------------------------------------------------------------

async function dispatchGameExited(
  page: Page,
  instanceId: string,
  outcome: 'success' | 'crash' | 'cancelled' | 'unknown',
  snapshotId: string | null,
  exitCode: number | null = 0,
) {
  await page.evaluate(
    ({ instanceId, outcome, snapshotId, exitCode }) => {
      const listeners = (window as any).__tauriEventListeners as Map<string, number>;
      const callbacks = (window as any).__callbacks as Map<number, (...args: unknown[]) => void>;
      const handlerId = listeners.get('game-exited');
      if (handlerId != null && callbacks) {
        const cb = callbacks.get(handlerId);
        if (cb) cb({ payload: { instance_id: instanceId, exit_code: exitCode, outcome, snapshot_id: snapshotId } });
      }
    },
    { instanceId, outcome, snapshotId, exitCode },
  );
}

// ---------------------------------------------------------------------------
// Mock installer — health + launch flow
// All data is inlined to avoid module-scope references inside addInitScript.
// ---------------------------------------------------------------------------

interface LaunchEventMockOptions {
  direct: boolean;
  healthBlockers?: boolean;
  failLaunch?: boolean;
}

async function installLaunchEventMock(page: Page, options: LaunchEventMockOptions) {
  await page.addInitScript(({ options }) => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const commandCalls: Record<string, number> = {};
    const eventListeners = new Map<string, number>();

    const row = {
      instance_id: 'launch-test-instance',
      name: 'Launch Test',
      loader: 'fabric',
      loader_version: '0.16',
      minecraft_version: '1.21',
      is_locked: false,
      last_launched_at: null,
    };

    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        commandCalls[command] = (commandCalls[command] ?? 0) + 1;

        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'launch_mode') return Promise.resolve(options.direct ? 'direct' : 'delegation');
          return Promise.resolve(false);
        }
        if (command === 'set_setting') return Promise.resolve(null);
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
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

        if (command === 'plugin:event|listen') {
          eventListeners.set(args.event as string, args.handler as number);
          return Promise.resolve(1);
        }
        if (command === 'plugin:event|unlisten') return Promise.resolve(1);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);

        // Multi-session launch state is a list; the backend never returns null.
        if (command === 'query_launch_state') return Promise.resolve([]);
        if (command === 'list_instances') return Promise.resolve([row]);

        if (command === 'check_instance_crash') return Promise.resolve(null);
        if (command === 'check_instance_health') {
          return Promise.resolve({
            score: options.healthBlockers ? 'red' : 'green',
            blockers: options.healthBlockers
              ? [{ kind: 'incompatible_mod', mod_id: 'blocker-mod', filename: 'blocker.jar', message: 'This mod is incompatible', suggested_action: null }]
              : [],
            warnings: [],
          });
        }

        if (command === 'launch_instance_direct') {
          if (options.failLaunch) return Promise.reject(new Error('Direct launch failure: JVM exited with code 1'));
          return Promise.resolve(4242);
        }

        if (command === 'launch_instance') {
          if (options.failLaunch) return Promise.reject(new Error('Delegated launch failure'));
          return Promise.resolve(null);
        }

        if (command === 'list_categories') return Promise.resolve([]);
        if (command === 'list_manifest_loaders') return Promise.resolve([]);
        if (command === 'list_manifest_mc_versions') return Promise.resolve([]);
        if (command === 'for_you_items') return Promise.resolve([]);
        if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
        if (command === 'get_lkg_marker') return Promise.resolve(null);
        if (command === 'check_instance_updates') return Promise.resolve([]);

        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __commandCalls: commandCalls,
      __tauriEventListeners: eventListeners,
      __callbacks: callbacks,
    });
  }, { options });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe('Launch events — successful outcome / LKG refresh', () => {

  test('direct launch shows running then exited with success outcome and snapshotId', async ({ page }) => {
    await installLaunchEventMock(page, {
      direct: true,
      healthBlockers: false,
      failLaunch: false,
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();

    // Launch the instance — health all-clear so it goes straight to running
    await page.getByRole('button', { name: 'Launch' }).first().click();
    await expect(page.getByText(/Running \(PID 4242\)/)).toBeVisible();

    // Verify launch_instance_direct was called once
    const directCalls = await page.evaluate(() => {
      const c = (window as any).__commandCalls as Record<string, number>;
      return c['launch_instance_direct'] ?? 0;
    });
    expect(directCalls).toBe(1);

    // Check that check_instance_health was called (all-clear path)
    const healthCalls = await page.evaluate(() => {
      const c = (window as any).__commandCalls as Record<string, number>;
      return c['check_instance_health'] ?? 0;
    });
    expect(healthCalls).toBe(1);

    // Now dispatch the game-exited event with success outcome and a snapshot ID (LKG promotion)
    await dispatchGameExited(page, 'launch-test-instance', 'success', 'snap-lkg-promoted', 0);

    // After game-exited, the Running indicator should clear
    await expect(page.getByText(/Running \(PID/)).toHaveCount(0);

    // The instance card reverts to "Never launched" (our mock has last_launched_at=null)
    await expect(page.getByText('Never launched')).toBeVisible();
  });

  test('delegated launch transitions to delegated then exited on game-exited event', async ({ page }) => {
    await installLaunchEventMock(page, {
      direct: false,
      healthBlockers: false,
      failLaunch: false,
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();

    // Launch delegated
    await page.getByRole('button', { name: 'Launch' }).first().click();

    // Delegated launches show no PID
    await expect(page.getByText(/Running \(PID/)).toHaveCount(0);

    // Verify launch_instance (delegated) was called
    const delegatedCalls = await page.evaluate(() => {
      const c = (window as any).__commandCalls as Record<string, number>;
      return c['launch_instance'] ?? 0;
    });
    expect(delegatedCalls).toBe(1);

    // Dispatch game-exited
    await dispatchGameExited(page, 'launch-test-instance', 'success', 'snap-lkg-delegated', 0);

    // Still shows "Never launched" since our mock doesn't update last_launched_at
    await expect(page.getByText('Never launched')).toBeVisible();
  });

  test('crash outcome from game-exited still clears running state', async ({ page }) => {
    await installLaunchEventMock(page, {
      direct: true,
      healthBlockers: false,
      failLaunch: false,
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Launch' }).first().click();
    await expect(page.getByText(/Running \(PID/)).toBeVisible();

    // Dispatch game-exited with crash outcome (exit code 1, no snapshot)
    await dispatchGameExited(page, 'launch-test-instance', 'crash', null, 1);

    // Running state clears
    await expect(page.getByText(/Running \(PID/)).toHaveCount(0);
  });

});

test.describe('Launch events — failed launch non-promotion', () => {

  test('health check failure keeps dialog open and recoverable', async ({ page }) => {
    await installLaunchEventMock(page, {
      direct: true,
      healthBlockers: true,
      failLaunch: false,
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Launch' }).first().click();

    // Health dialog should appear (because of blockers)
    await expect(page.getByText('This mod is incompatible')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Cancel' })).toBeEnabled();

    // The instance does NOT show Running — the launch was not promoted
    await expect(page.getByText(/Running \(PID/)).toHaveCount(0);

    // verify no launch command was executed
    const directCalls = await page.evaluate(() => {
      const c = (window as any).__commandCalls as Record<string, number>;
      return c['launch_instance_direct'] ?? 0;
    });
    expect(directCalls).toBe(0);
  });

  test('direct launch failure stays in failed state without promoting to running', async ({ page }) => {
    await installLaunchEventMock(page, {
      direct: true,
      healthBlockers: false,
      failLaunch: true,
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Launch' }).first().click();

    // Health all-clear → no dialog, but launch_instance_direct throws
    await expect(page.getByText('Direct launch failure: JVM exited with code 1')).toBeVisible();

    // The error is shown on the instance card
    await expect(page.getByRole('button', { name: 'Dismiss' })).toBeVisible();

    // Never shows Running state
    await expect(page.getByText(/Running \(PID/)).toHaveCount(0);

    // After dismissing, launch button is enabled again (recoverable)
    await page.getByRole('button', { name: 'Dismiss' }).click();
    await expect(page.getByRole('button', { name: 'Launch' }).first()).toBeEnabled();
  });

  test('health dialog cancel does not launch', async ({ page }) => {
    await installLaunchEventMock(page, {
      direct: true,
      healthBlockers: true,
      failLaunch: false,
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Launch' }).first().click();

    // Health dialog appears
    await expect(page.getByText('This mod is incompatible')).toBeVisible();

    // Cancel
    await page.getByRole('button', { name: 'Cancel' }).click();

    // No launch was performed
    const directCallsAfterCancel = await page.evaluate(() => {
      const c = (window as any).__commandCalls as Record<string, number>;
      return c['launch_instance_direct'] ?? 0;
    });
    expect(directCallsAfterCancel).toBe(0);

    // Instance is back to idle — Launch button enabled
    await expect(page.getByRole('button', { name: 'Launch' }).first()).toBeEnabled();
  });

});
