import { test, expect, type Page } from '@playwright/test';

async function installHealthMock(page: Page, options: {
  direct: boolean;
  blocker?: boolean;
  filename?: string | null;
  failLaunch?: boolean;
  msaRequired?: boolean;
  mutedWarning?: boolean;
  recommendationOnly?: boolean;
  loaderIssue?: Record<string, unknown>;
  failRecovery?: boolean;
  periodicBlocker?: boolean;
}) {
  await page.addInitScript(({ options }) => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const counts: Record<string, number> = {};
    const lastArgs: Record<string, Record<string, unknown>> = {};
    const eventListeners = new Map<string, (payload: unknown) => void>();
    const row = {
      instance_id: 'health-test', name: 'Health Test', loader: 'fabric',
      loader_version: '0.16', minecraft_version: '1.21', is_locked: false,
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
        counts[command] = (counts[command] ?? 0) + 1;
        lastArgs[command] = args;
        if (command === 'get_setting') {
          if (args.key === 'onboarding_complete') return Promise.resolve(true);
          if (args.key === 'launch_mode') return Promise.resolve(options.direct ? 'direct' : 'delegation');
          if (args.key === 'health_preferences' && options.mutedWarning) {
            return Promise.resolve({
              mutedWarnings: ['unknown_mod:example:a75df095'],
              muteAllRecommendations: false,
            });
          }
          return Promise.resolve(false);
        }
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'plugin:event|listen') {
          eventListeners.set(args.event as string, args.handler as number);
          return Promise.resolve(1);
        }
        if (command === 'plugin:event|unlisten') return Promise.resolve(1);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command === 'list_instances') return Promise.resolve([row]);
        if (command === 'check_instance_crash') return Promise.resolve(null);
        if (command === 'check_instance_health') {
          return Promise.resolve({
            score: options.blocker ? 'red' : options.recommendationOnly ? 'green' : 'yellow',
            blockers: options.blocker ? (options.loaderIssue ? [{
              kind: 'loader_version_mismatch', mod_id: null,
              filename: null,
              message: 'The installed fabric loader 0.16 does not satisfy the loader requirements of enabled mods.',
              suggested_action: 'Switch the instance loader to fabric 0.19.0.',
              loader_compatibility: options.loaderIssue,
            }] : [{
              kind: 'incompatible_mod', mod_id: 'example',
              filename: options.filename === undefined ? 'example.jar' : options.filename,
              message: 'Example blocker', suggested_action: null,
            }]) : [],
            warnings: options.blocker || options.recommendationOnly ? [] : [{
              kind: 'unknown_mod', mod_id: 'example', filename: 'example.jar',
              message: 'Example warning', suggested_action: null,
            }],
            recommendations: options.recommendationOnly ? [{
              kind: 'missing_optional_dependency', mod_id: 'rei', source_filename: 'example.jar',
              message: 'Example recommendation', suggested_action: null,
            }] : [],
            scan_token: 'health-e2e-scan',
          });
        }
        if (command === 'check_all_instance_health') {
          if (!options.periodicBlocker) return Promise.resolve([]);
          return Promise.resolve([{
            instance_id: row.instance_id,
            report: {
              score: 'red',
              blockers: [{
                kind: 'missing_required_dependency', mod_id: 'fabric-api', filename: 'example.jar',
                message: "'example.jar' requires 'fabric-api' but no enabled artifact provides it.",
                suggested_action: "Install 'fabric-api' to resolve this dependency.",
              }],
              warnings: [],
              recommendations: [],
              scan_token: 'periodic-health-scan',
            },
            error: null,
          }]);
        }
        if (command === 'launch_instance_with_recovery') {
          return options.failRecovery
            ? Promise.reject(new Error('Switch failed'))
            : Promise.resolve(4242);
        }
        if (command === 'launch_instance_direct') {
          if (options.msaRequired) {
            return Promise.reject({
              code: 'ERR_MSA_AUTH_REQUIRED',
              message: 'Sign in with Microsoft to use direct launch.',
              details: null,
              suggested_action: 'Sign in with Microsoft',
            });
          }
          return options.failLaunch ? Promise.reject(new Error('Launch failed')) : Promise.resolve(4242);
        }
        if (command === 'launch_instance') return Promise.resolve(null);
        if (command === 'disable_mod_for_test') return Promise.resolve(null);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __commandCounts: counts,
      __lastCommandArgs: lastArgs,
      __tauriEventListeners: eventListeners,
      __callbacks: callbacks,
    });
  }, { options });
}

function installExitEventMock(page: Page, instanceId: string) {
  return page.evaluate((instanceId) => {
    const listeners = (window as any).__tauriEventListeners as Map<string, number>;
    const callbacks = (window as any).__callbacks as Map<number, (...args: unknown[]) => void>;
    const handlerId = listeners.get('game-exited');
    if (handlerId != null && callbacks) {
      const cb = callbacks.get(handlerId);
      if (cb) cb({ payload: { instance_id: instanceId, exit_code: 0 } });
    }
  }, instanceId);
}

test('health approval preserves direct launch and scans only once', async ({ page }) => {
  await installHealthMock(page, { direct: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();

  await expect(page.getByText('Example warning')).toBeVisible();
  expect(await page.evaluate(() => (window as any).__commandCounts.check_instance_health)).toBe(1);
  await page.getByRole('button', { name: 'Launch Anyway' }).click();

  await expect(page.getByText(/Running \(PID 4242\)/)).toBeVisible();
  expect(await page.evaluate(() => (window as any).__commandCounts.launch_instance_direct)).toBe(1);
  expect(await page.evaluate(() => (window as any).__lastCommandArgs.launch_instance_direct.healthScanToken)).toBe('health-e2e-scan');
  expect(await page.evaluate(() => (window as any).__commandCounts.launch_instance ?? 0)).toBe(0);
});

test('health approval preserves delegated launch without a fake PID', async ({ page }) => {
  await installHealthMock(page, { direct: false });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();
  await page.getByRole('button', { name: 'Launch Anyway' }).click();

  await expect(page.getByText('Example warning')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__commandCounts.launch_instance)).toBe(1);
  await expect(page.getByText(/Running \(PID/)).toHaveCount(0);
});

test('cancel performs no launch and filename-backed disable uses the filename', async ({ page }) => {
  await installHealthMock(page, { direct: true, blocker: true, filename: 'example.jar' });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();
  await page.getByRole('button', { name: 'Disable' }).click();
  expect(await page.evaluate(() => (window as any).__lastCommandArgs.disable_mod_for_test.filename)).toBe('example.jar');
  await page.getByRole('button', { name: 'Cancel' }).click();
  expect(await page.evaluate(() => (window as any).__commandCounts.launch_instance_direct ?? 0)).toBe(0);
});

test('finding without filename has no Disable action', async ({ page }) => {
  await installHealthMock(page, { direct: true, blocker: true, filename: null });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();
  await expect(page.getByRole('button', { name: 'Disable' })).toHaveCount(0);
});

test('periodic all-instance health scan alerts the card and reuses repair dialog', async ({ page }) => {
  await installHealthMock(page, { direct: true, periodicBlocker: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();

  await expect(page.getByRole('alert', { name: '1 health issue detected' })).toBeVisible();
  await page.getByRole('button', { name: 'Review & repair' }).click();

  await expect(page.getByRole('heading', { name: 'Health Check' })).toBeVisible();
  await expect(page.getByText("'example.jar' requires 'fabric-api' but no enabled artifact provides it.")).toBeVisible();
  await page.getByRole('button', { name: 'Disable' }).click();
  expect(await page.evaluate(() => (window as any).__lastCommandArgs.disable_mod_for_test.filename)).toBe('example.jar');
  expect(await page.evaluate(() => (window as any).__commandCounts.launch_instance_direct ?? 0)).toBe(0);
});

test('game-exited event clears running state in UI', async ({ page }) => {
  await installHealthMock(page, { direct: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();
  await page.getByRole('button', { name: 'Launch Anyway' }).click();
  await expect(page.getByText(/Running \(PID/)).toBeVisible();

  await installExitEventMock(page, 'health-test');

  await expect(page.getByText(/Running \(PID/)).toHaveCount(0);
  await expect(page.getByText('Never launched')).toBeVisible();
});

test('failed launch keeps the dialog recoverable', async ({ page }) => {
  await installHealthMock(page, { direct: true, failLaunch: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();
  await page.getByRole('button', { name: 'Launch Anyway' }).click();
  await expect(page.getByText('Launch failed').first()).toBeVisible();
  await expect(page.getByRole('button', { name: 'Cancel' })).toBeEnabled();
});

test('all muted warnings skip the health dialog', async ({ page }) => {
  await installHealthMock(page, { direct: true, mutedWarning: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();

  await expect(page.getByRole('heading', { name: 'Health Check' })).toHaveCount(0);
  await expect(page.getByText(/Running \(PID 4242\)/)).toBeVisible();
});

test('recommendation-only health report does not interrupt launch', async ({ page }) => {
  await installHealthMock(page, { direct: true, recommendationOnly: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();

  await expect(page.getByText('Example recommendation')).toHaveCount(0);
  await expect(page.getByText(/Running \(PID 4242\)/)).toBeVisible();
});

test('microsoft auth error releases the health dialog', async ({ page }) => {
  await installHealthMock(page, { direct: true, msaRequired: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();
  await page.getByRole('button', { name: 'Launch Anyway' }).click();

  await expect(page.getByRole('heading', { name: 'Health Check' })).toHaveCount(0);
  await expect(page.getByText('Sign in with Microsoft to use direct launch.').first()).toBeVisible();
});

// ---------------------------------------------------------------------------
// Loader compatibility recovery (Work Package 7)
// ---------------------------------------------------------------------------

const loaderIssueFixture: Record<string, unknown> = {
  loader: 'fabric',
  current_version: '0.16',
  recommended_version: '0.19.0',
  compatible_versions: ['0.19.0', '0.18.6'],
  requirements: [{
    declaring_mod_id: 'moda',
    target_id: 'fabricloader',
    version_ranges: ['>=0.19.0'],
    importance: 'required',
    candidate_version: '0.16',
    verdict: 'unsatisfied',
  }],
  conflicts: [],
};

const loaderIssueNoCandidatesFixture: Record<string, unknown> = {
  loader: 'fabric',
  current_version: '0.16',
  recommended_version: null,
  compatible_versions: [],
  requirements: [{
    declaring_mod_id: 'moda',
    target_id: 'fabricloader',
    version_ranges: ['>=0.19.0'],
    importance: 'required',
    candidate_version: '0.16',
    verdict: 'unsatisfied',
  }],
  conflicts: [{
    declaring_mod_id: 'moda',
    target_id: 'fabricloader',
    version_ranges: ['>=0.19.0'],
    with_declaring_mod_id: 'modb',
    with_target_id: 'fabricloader',
    with_version_ranges: ['<0.19.0'],
    message: 'conflict between moda and modb: no signed loader candidate satisfies both',
  }],
};

test('loader blocker renders recommendation and Switch and launch uses exactly one recovery call', async ({ page }) => {
  await installHealthMock(page, { direct: true, blocker: true, loaderIssue: loaderIssueFixture });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();

  await expect(page.getByText('Loader: fabric 0.16')).toBeVisible();
  await expect(page.getByText('Recommended version: 0.19.0')).toBeVisible();

  await page.getByRole('button', { name: 'Switch and launch' }).click();
  await expect(page.getByText(/Running \(PID 4242\)/)).toBeVisible();

  const counts = await page.evaluate(() => (window as any).__commandCounts);
  const args = await page.evaluate(() => (window as any).__lastCommandArgs.launch_instance_with_recovery);
  expect(counts.launch_instance_with_recovery).toBe(1);
  expect(counts.launch_instance_direct ?? 0).toBe(0);
  expect(args.instanceId).toBe('health-test');
  expect(args.action).toEqual({ type: 'SwitchLoader', target_version: '0.19.0' });
  expect(args.allowHealthBlockers).toBe(false);
  expect(args.healthScanToken).toBe('health-e2e-scan');
});

test('loader blocker Choose compatible version switches to the selected candidate only', async ({ page }) => {
  await installHealthMock(page, { direct: true, blocker: true, loaderIssue: loaderIssueFixture });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();

  await page.getByRole('combobox', { name: 'Choose compatible version' }).click();
  await page.getByRole('option', { name: '0.18.6' }).click();
  await expect(page.getByText(/Running \(PID 4242\)/)).toBeVisible();

  const counts = await page.evaluate(() => (window as any).__commandCounts);
  const args = await page.evaluate(() => (window as any).__lastCommandArgs.launch_instance_with_recovery);
  expect(counts.launch_instance_with_recovery).toBe(1);
  expect(args.action).toEqual({ type: 'SwitchLoader', target_version: '0.18.6' });
});

test('loader blocker View requirements discloses raw predicates and declarer', async ({ page }) => {
  await installHealthMock(page, { direct: true, blocker: true, loaderIssue: loaderIssueFixture });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();

  await page.getByText('View requirements (1)').click();
  await expect(page.getByText(/moda → fabricloader/)).toBeVisible();
  await expect(page.getByText('Predicates: >=0.19.0')).toBeVisible();
  await expect(page.getByText(/installed candidate provides 0.16/)).toBeVisible();
});

test('loader blocker with no compatible candidate surfaces conflicts and no switch action', async ({ page }) => {
  await installHealthMock(page, { direct: true, blocker: true, loaderIssue: loaderIssueNoCandidatesFixture });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();

  await expect(page.getByText('No signed loader version satisfies every enabled mod requirement. Review the conflicting requirements below; automatic switching is not available.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Switch and launch' })).toHaveCount(0);
  await expect(page.getByRole('combobox', { name: 'Choose compatible version' })).toHaveCount(0);

  await page.getByText('View requirements (1) · 1 conflict').click();
  await expect(page.getByText('conflict between moda and modb: no signed loader candidate satisfies both')).toBeVisible();
});

test('loader blocker switch failure keeps the dialog open with the error visible', async ({ page }) => {
  await installHealthMock(page, { direct: true, blocker: true, loaderIssue: loaderIssueFixture, failRecovery: true });
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Launch' }).first().click();

  await page.getByRole('button', { name: 'Switch and launch' }).click();

  await expect(page.getByRole('heading', { name: 'Health Check' })).toBeVisible();
  await expect(page.getByText('Switch failed')).toBeVisible();
  expect(await page.evaluate(() => (window as any).__commandCounts.launch_instance_with_recovery)).toBe(1);
  await expect(page.getByRole('button', { name: 'Cancel' })).toBeEnabled();
});
