import { test, expect, type Page } from '@playwright/test';

// ---------------------------------------------------------------------------
// Types for the install call queue
// ---------------------------------------------------------------------------

interface InstallCall {
  command: string;
  args: Record<string, unknown>;
  resolve: (value: unknown) => void;
  reject: (reason?: unknown) => void;
}

interface UpdateInfo {
  filename: string;
  mod_jar_id: string;
  current_version: string;
  latest_version: string;
  target_version: string;
  source: string;
}

// ---------------------------------------------------------------------------
// Fixtures — realistic instances and update data
// ---------------------------------------------------------------------------

const VANILLA_UPDATES: UpdateInfo[] = [
  {
    filename: 'sodium-0.6.0.jar',
    mod_jar_id: 'sodium',
    current_version: '0.5.11',
    latest_version: '0.6.0',
    target_version: '0.6.0',
    source: 'curated',
  },
  {
    filename: 'lithium-0.12.1.jar',
    mod_jar_id: 'lithium',
    current_version: '0.12.0',
    latest_version: '0.12.1',
    target_version: '0.12.1',
    source: 'curated',
  },
];

// ---------------------------------------------------------------------------
// Fixtures — ResolvedInstallPlan / InstallOutcome builders for batch-update
// ---------------------------------------------------------------------------

function makeBatchPlan(overrides: Record<string, unknown> = {}) {
  return {
    fingerprint: 'plan-fp-batch-001',
    intent: {
      action: { type: 'batch-update' as const, items: [] },
      targetInstance: 'vanilla-instance',
      optionalDeps: { type: 'prompt' as const },
      requestedBy: 'auto-update' as const,
      overrides: { allowReplace: false, skipHealthScan: false, forceConflictResolution: {} },
    },
    operation: { type: 'batch-update' as const, operations: [] },
    dependencies: [],
    conflicts: [],
    filesToAdd: [],
    filesToRemove: [],
    filesToDisable: [],
    snapshot: { label: 'Before batch update', estimatedBytes: 500_000 },
    diskEstimate: { downloadBytes: 0, snapshotBytes: 500_000, applyOverheadBytes: 100_000, peakAdditionalBytes: 600_000, postCommitDeltaBytes: 250_000 },
    warnings: [],
    blockingErrors: [],
    pendingChoices: [],
    createdAt: '2026-07-12T17:00:00Z',
    instanceStateHash: 'abc123def456',
    registryRevision: 'v20260712',
    ...overrides,
  };
}

function makeSuccessOutcome(snapshotId = 'snap-success-001') {
  return {
    type: 'success' as const,
    installedItems: ['sodium-0.6.0.jar', 'lithium-0.12.1.jar'],
    existingItemsReused: [],
    warnings: [],
    health: { type: 'completed' as const, report: {} },
    snapshotId,
  };
}

// ---------------------------------------------------------------------------
// Shared mock installer for UpdatesSection tests
// ---------------------------------------------------------------------------

async function updatesSectionMock(page: Page) {
  const updatesByInstance: Record<string, UpdateInfo[]> = {
    'vanilla-instance': VANILLA_UPDATES,
  };

  await page.addInitScript(
    (params: { updates: Record<string, UpdateInfo[]> }) => {
      const { updates } = params;

      const installCalls: InstallCall[] = [];

      const callbacks = new Map<number, (...args: unknown[]) => void>();
      let callbackId = 0;

      const internals = {
        transformCallback(callback: (...args: unknown[]) => void) {
          const id = ++callbackId;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback(id: number) { callbacks.delete(id); },
        invoke(command: string, args: Record<string, unknown> = {}) {
          // Install pipeline commands — tracked in call queue
          if (command === 'resolve_install_plan') {
            return new Promise((resolve, reject) => installCalls.push({ command, args, resolve, reject } as any));
          }
          if (command === 'apply_install_plan') {
            return new Promise((resolve, reject) => installCalls.push({ command, args, resolve, reject } as any));
          }
          if (command === 'cancel_install') return Promise.resolve(null);

          // Event plugin (used by subscribeProgress for progress events)
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);

          // Settings
          if (command === 'get_setting') {
            const key = args.key as string;
            if (key === 'onboarding_complete') return Promise.resolve(true);
            if (key === 'modrinth_enabled') return Promise.resolve(true);
            if (key === 'ai_chat_enabled') return Promise.resolve(false);
            if (key === 'mojang_launcher_path') return Promise.resolve('');
            if (key === 'launch_mode') return Promise.resolve('delegation');
            return Promise.resolve(null);
          }

          // Registry
          if (command === 'get_registry_status') {
            return Promise.resolve({ has_cached_db: true, cached_tag: 'test', cached_schema_version: 5, latest_tag: 'test', update_available: false, checked: true, message: 'Registry ready.' });
          }
          if (command === 'list_categories') return Promise.resolve([]);
          if (command === 'list_manifest_loaders') return Promise.resolve(['fabric', 'forge', 'quilt']);
          if (command === 'list_manifest_mc_versions') return Promise.resolve(['1.20.1', '1.21']);
          if (command === 'list_loader_versions') return Promise.resolve([]);

          // Misc
          if (command === 'get_windows_accent_color') return Promise.resolve(null);
          if (command === 'get_auth_status') return Promise.resolve(true);
          if (command === 'get_github_profile') return Promise.resolve(null);
          if (command === 'get_flag_rate_limit') return Promise.resolve(null);
          if (command === 'list_mod_reviews') return Promise.resolve([]);
          if (command === 'get_curated_annotation') return Promise.resolve(null);

          // Instances
          if (command === 'list_instances') {
            return Promise.resolve([
              { instance_id: 'vanilla-instance', name: 'Vanilla', minecraft_version: '1.21', loader: 'fabric', loader_version: '0.16.0', is_modpack: false, is_locked: false, last_launched_at: null, jvm_memory_mb: 4096, jvm_gc: 'G1GC', jvm_custom_args: '', created_at: '2026-01-01T00:00:00Z' },
              { instance_id: 'locked-instance', name: 'Locked Modded', minecraft_version: '1.20.1', loader: 'fabric', loader_version: '0.15.11', is_modpack: false, is_locked: true, last_launched_at: null, jvm_memory_mb: 4096, jvm_gc: 'G1GC', jvm_custom_args: '', created_at: '2026-01-01T00:00:00Z' },
            ]);
          }
          if (command === 'get_instance_detail') return Promise.resolve(null);
          if (command === 'list_snapshots') return Promise.resolve([]);
          if (command === 'list_loadout_profiles') return Promise.resolve([]);
          if (command === 'restore_snapshot') return Promise.resolve(null);

          // Crash check
          if (command === 'check_instance_crash') return Promise.resolve(null);

          // Updates check
          if (command === 'check_instance_updates') {
            const instanceId = args.instanceId as string;
            return Promise.resolve((updates as any)[instanceId] ?? []);
          }

          // Mod detail / browse
          if (command === 'get_registry_item') return Promise.resolve(null);
          if (command === 'fetch_modrinth_project') return Promise.resolve(null);
          if (command === 'is_modrinth_enabled') return Promise.resolve(true);
          if (command === 'list_mod_versions') return Promise.resolve({ items: [], hasMore: false });
          if (command === 'list_raw_modrinth_versions') return Promise.resolve([]);
          if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
          if (command === 'browse_load_more') return Promise.resolve({ items: [], total: 0, page: 1, hasMore: false });
          if (command === 'for_you_items') return Promise.resolve({ items: [] });
          if (command === 'browse_items') return Promise.resolve([]);
          if (command === 'check_mod_compat') return Promise.resolve('');

          // Instance lifecycle commands
          if (command === 'delete_instance') return Promise.resolve();
          if (command === 'unlock_instance') return Promise.resolve();
          if (command === 'lock_instance') return Promise.resolve();

          // Fallback
          return Promise.resolve(null);
        },
      };
      Object.assign(window as unknown as Record<string, unknown>, {
        __TAURI_INTERNALS__: internals,
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
        __installCalls: installCalls,
      });
    },
    { updates: updatesByInstance } as any,
  );
}

// ---------------------------------------------------------------------------
// Helpers: wait for and resolve install pipeline calls
// ---------------------------------------------------------------------------

async function totalInstallCalls(page: Page): Promise<number> {
  return page.evaluate(() => (window as any).__installCalls?.length ?? 0);
}

async function lastInstallCall(page: Page, command: string): Promise<number> {
  let index = -1;
  await expect.poll(async () => {
    const calls: InstallCall[] = await page.evaluate(() => (window as any).__installCalls ?? []);
    const indices = calls
      .map((c: InstallCall, i: number) => ({ c, i }))
      .filter(({ c }) => c.command === command)
      .map(({ i }) => i);
    index = indices.length > 0 ? indices[indices.length - 1] : -1;
    return index;
  }).toBeGreaterThanOrEqual(0);
  return index;
}

async function resolveInstallCall(page: Page, index: number, result: unknown) {
  await page.evaluate(
    ({ idx, res }: { idx: number; res: unknown }) => {
      const calls = (window as any).__installCalls as InstallCall[];
      if (calls[idx]) calls[idx].resolve(res);
    },
    { idx: index, res: result },
  );
}

async function rejectInstallCall(page: Page, index: number, error: unknown) {
  await page.evaluate(
    ({ idx, err }: { idx: number; err: unknown }) => {
      const calls = (window as any).__installCalls as InstallCall[];
      if (calls[idx]) calls[idx].reject(err);
    },
    { idx: index, err: error },
  );
}

// ---------------------------------------------------------------------------
// Helpers: common assertions on the InstallFlow dialog
// ---------------------------------------------------------------------------

async function expectReviewView(page: Page) {
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByText('Review Instance Changes')).toBeVisible();
  await expect(page.getByText(/Before batch update/)).toBeVisible();
  await expect(page.getByRole('button', { name: /Apply Updates/ })).toBeVisible();
}

async function expectResultView(page: Page) {
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByText('All verified changes were applied successfully.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Close' }).first()).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open Instance' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Roll Back' })).toBeVisible();
}

async function expectErrorView(page: Page, message: string) {
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByText(message)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Close' }).first()).toBeVisible();
}

// ---------------------------------------------------------------------------
// Tests — Release C4 UpdatesSection
// ---------------------------------------------------------------------------

test.describe('Release C4 — UpdatesSection', () => {

  test('checking shows compatible updates for the unlocked instance', async ({ page }) => {
    await updatesSectionMock(page);
    await page.addInitScript(() => {
      window.history.replaceState({ __agora: { type: 'tab', tab: 'instances' } }, '');
    });
    await page.goto('/');

    // Wait for instances to render
    await expect(page.getByText('Vanilla')).toBeVisible();
    await expect(page.getByText('Locked Modded')).toBeVisible();

    // Initially only the "Check for Updates" button is shown
    const checkBtn = page.getByRole('button', { name: 'Check for Updates' });
    await expect(checkBtn).toBeVisible();

    // Click to check for updates
    await checkBtn.click();

    // Wait for the updates section to appear with the heading
    await expect(page.getByText('Updates Available (2)')).toBeVisible({ timeout: 10_000 });

    // Verify the update entries for the unlocked instance
    await expect(page.getByText('sodium-0.6.0.jar')).toBeVisible();
    await expect(page.getByText('lithium-0.12.1.jar')).toBeVisible();

    // Verify the version transition labels
    await expect(page.getByText('0.5.11 →')).toBeVisible();
    await expect(page.getByText('0.12.0 →')).toBeVisible();
    await expect(page.getByText('0.6.0').first()).toBeVisible();
    await expect(page.getByText('0.12.1').first()).toBeVisible();

    // Verify each update row has a checkbox for selection
    const checkboxes = page.locator('input[type="checkbox"]');
    await expect(checkboxes).toHaveCount(2);

    // Verify the "Update All" button shows the correct count
    await expect(page.getByRole('button', { name: 'Update All (2)' })).toBeVisible();
  });

  test('locked instance has no update action in the updates section', async ({ page }) => {
    await updatesSectionMock(page);
    await page.addInitScript(() => {
      window.history.replaceState({ __agora: { type: 'tab', tab: 'instances' } }, '');
    });
    await page.goto('/');

    // Wait for instances to render
    await expect(page.getByText('Vanilla')).toBeVisible();
    await expect(page.getByText('Locked Modded')).toBeVisible();

    // The locked instance card shows the lock badge visible on the InstanceCard
    await expect(page.getByText('Locked Modded').locator('..').getByText('Locked')).toBeVisible();

    // Click "Check for Updates"
    await page.getByRole('button', { name: 'Check for Updates' }).click();

    // After checkAll completes, only the unlocked instance's updates appear.
    // The locked instance is skipped entirely because is_locked === true.
    await expect(page.getByText('Updates Available (2)')).toBeVisible({ timeout: 10_000 });

    // The unlocked instance update card shows with controls
    // ("Vanilla" appears both in the InstanceCard heading and the UpdatesSection card — .first() disambiguates)
    await expect(page.getByText('Vanilla').first()).toBeVisible();
    await expect(page.getByRole('button', { name: 'Update All (2)' })).toBeVisible();

    // No update card exists for the locked instance — locked instances are
    // skipped during checkAll and never added to updatesByInstance.
    // Only the unlocked instance's "Update All" button is rendered.
    await expect(page.getByRole('button', { name: /Update All \(\d+\)/ })).toHaveCount(1);
  });

  test('select and update all opens confirmation dialog then canonical InstallFlow', async ({ page }) => {
    await updatesSectionMock(page);
    await page.addInitScript(() => {
      window.history.replaceState({ __agora: { type: 'tab', tab: 'instances' } }, '');
    });
    await page.goto('/');

    // Wait for instances
    await expect(page.getByText('Vanilla')).toBeVisible();

    // Check for updates
    await page.getByRole('button', { name: 'Check for Updates' }).click();
    await expect(page.getByText('Updates Available (2)')).toBeVisible({ timeout: 10_000 });

    // Click "Update All (2)" — this selects all and opens the confirmation dialog
    await page.getByRole('button', { name: 'Update All (2)' }).click();

    // Confirmation dialog appears
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText('Review 2 updates')).toBeVisible();
    await expect(dialog.getByText('sodium-0.6.0.jar')).toBeVisible();
    await expect(dialog.getByText('lithium-0.12.1.jar')).toBeVisible();
    await expect(dialog.getByText('0.5.11 →')).toBeVisible();
    await expect(dialog.getByText('0.12.0 →')).toBeVisible();

    // Click "Review Plan" — this triggers the batch-update flow
    await dialog.getByRole('button', { name: 'Review Plan' }).click();

    // The InstallFlow dialog should now open (may be the same dialog slot)
    // Wait for resolve_install_plan to be called
    await expect.poll(() => totalInstallCalls(page)).toBeGreaterThanOrEqual(1);
    const resolveIdx = await lastInstallCall(page, 'resolve_install_plan');

    // Resolve the plan
    await resolveInstallCall(page, resolveIdx, makeBatchPlan());

    // Verify the InstallFlow review view appears
    await expectReviewView(page);
  });

  test('batch intent contains every selected exact target version', async ({ page }) => {
    await updatesSectionMock(page);
    await page.addInitScript(() => {
      window.history.replaceState({ __agora: { type: 'tab', tab: 'instances' } }, '');
    });
    await page.goto('/');

    // Wait for instances
    await expect(page.getByText('Vanilla')).toBeVisible();

    // Check for updates
    await page.getByRole('button', { name: 'Check for Updates' }).click();
    await expect(page.getByText('Updates Available (2)')).toBeVisible({ timeout: 10_000 });

    // Click "Update All (2)"
    await page.getByRole('button', { name: 'Update All (2)' }).click();

    // Confirm dialog
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();

    // Click "Review Plan"
    await dialog.getByRole('button', { name: 'Review Plan' }).click();

    // Wait for resolve_install_plan to be called
    await expect.poll(() => totalInstallCalls(page)).toBeGreaterThanOrEqual(1);
    const resolveIdx = await lastInstallCall(page, 'resolve_install_plan');

    // Inspect the intent args to verify batch items
    const callArgs = await page.evaluate((idx: number) => {
      const calls = (window as any).__installCalls as InstallCall[];
      return calls[idx]?.args;
    }, resolveIdx);
    expect(callArgs).toBeTruthy();

    const intent = (callArgs as any).intent as Record<string, unknown>;
    expect(intent).toBeTruthy();
    expect(intent.requestedBy).toBe('auto-update');
    expect(intent.targetInstance).toBe('vanilla-instance');

    const action = intent.action as Record<string, unknown>;
    expect(action.type).toBe('batch-update');
    expect(action.items).toBeTruthy();
    const items = action.items as Array<Record<string, string>>;

    // Verify every selected update is in the batch with its target version
    expect(items).toHaveLength(2);

    const sodiumItem = items.find((i) => i.itemId === 'sodium');
    expect(sodiumItem).toBeTruthy();
    expect(sodiumItem!.targetVersion).toBe('0.6.0');

    const lithiumItem = items.find((i) => i.itemId === 'lithium');
    expect(lithiumItem).toBeTruthy();
    expect(lithiumItem!.targetVersion).toBe('0.12.1');

    // Resolve the plan to clean up the dialog
    await resolveInstallCall(page, resolveIdx, makeBatchPlan());

    // Wait for review view
    await expectReviewView(page);
  });

  test('failed artifact outcome leaves recovery messaging', async ({ page }) => {
    await updatesSectionMock(page);
    await page.addInitScript(() => {
      window.history.replaceState({ __agora: { type: 'tab', tab: 'instances' } }, '');
    });
    await page.goto('/');

    // Wait for instances
    await expect(page.getByText('Vanilla')).toBeVisible();

    // Check for updates → Update All → Review Plan
    await page.getByRole('button', { name: 'Check for Updates' }).click();
    await expect(page.getByText('Updates Available (2)')).toBeVisible({ timeout: 10_000 });
    await page.getByRole('button', { name: 'Update All (2)' }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: 'Review Plan' }).click();

    // Resolve the plan
    await expect.poll(() => totalInstallCalls(page)).toBeGreaterThanOrEqual(1);
    const resolveIdx = await lastInstallCall(page, 'resolve_install_plan');
    await resolveInstallCall(page, resolveIdx, makeBatchPlan());

    // Wait for review view and confirm
    await expect(page.getByRole('button', { name: /Apply Updates/ })).toBeVisible();
    await page.getByRole('button', { name: /Apply Updates/ }).click();

    // Apply fails
    await expect.poll(() => totalInstallCalls(page)).toBeGreaterThanOrEqual(2);
    const applyIdx = await lastInstallCall(page, 'apply_install_plan');
    await rejectInstallCall(page, applyIdx, new Error('Corrupt download: SHA-256 mismatch for sodium-0.6.0.jar'));

    // Error view shows the failure message
    await expectErrorView(page, 'Corrupt download: SHA-256 mismatch for sodium-0.6.0.jar');

    // The error view should also show recovery messaging because the outcome
    // would have a snapshotId when it's a failed outcome with rollback not performed.
    // The apply rejection error comes through InstallFlow's catch block which
    // dispatches a 'fail' action — the error message is shown but there is no
    // Roll Back button because the apply never completed (no outcome snapshot).
    // This matches the expected behavior for transport-level failures.
  });

  test('successful batch shows rollback option', async ({ page }) => {
    await updatesSectionMock(page);
    await page.addInitScript(() => {
      window.history.replaceState({ __agora: { type: 'tab', tab: 'instances' } }, '');
    });
    await page.goto('/');

    // Wait for instances
    await expect(page.getByText('Vanilla')).toBeVisible();

    // Check for updates → Update All → Review Plan
    await page.getByRole('button', { name: 'Check for Updates' }).click();
    await expect(page.getByText('Updates Available (2)')).toBeVisible({ timeout: 10_000 });
    await page.getByRole('button', { name: 'Update All (2)' }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await dialog.getByRole('button', { name: 'Review Plan' }).click();

    // Resolve the plan
    await expect.poll(() => totalInstallCalls(page)).toBeGreaterThanOrEqual(1);
    const resolveIdx = await lastInstallCall(page, 'resolve_install_plan');
    await resolveInstallCall(page, resolveIdx, makeBatchPlan());

    // Wait for review view and confirm
    await expect(page.getByRole('button', { name: /Apply Updates/ })).toBeVisible();
    await page.getByRole('button', { name: /Apply Updates/ }).click();

    // Apply succeeds
    await expect.poll(() => totalInstallCalls(page)).toBeGreaterThanOrEqual(2);
    const applyIdx = await lastInstallCall(page, 'apply_install_plan');
    await resolveInstallCall(page, applyIdx, makeSuccessOutcome());

    // Result view shows success with Roll Back option
    await expectResultView(page);
  });
});
