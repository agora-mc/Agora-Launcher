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
    filename: 'sodium-0.5.11.jar',
    mod_jar_id: 'sodium',
    current_version: '0.5.11',
    latest_version: '0.6.0',
    target_version: '0.6.0',
    source: 'curated',
  },
  {
    filename: 'lithium-0.12.0.jar',
    mod_jar_id: 'lithium',
    current_version: '0.12.0',
    latest_version: '0.12.1',
    target_version: '0.12.1',
    source: 'curated',
  },
];

const INSTALLED_MODS = [
  { filename: 'sodium-0.5.11.jar', display_name: 'Sodium', mod_jar_id: 'sodium', version: '0.5.11', sha256: 'a'.repeat(64) },
  { filename: 'lithium-0.12.0.jar', display_name: 'Lithium', mod_jar_id: 'lithium', version: '0.12.0', sha256: 'b'.repeat(64) },
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

function makeHealthRollbackOutcome(snapshotId = 'snap-health-001') {
  return {
    type: 'health-rollback' as const,
    snapshotId,
    healthReport: {
      blockers: [{ message: 'sodium 0.6.0 requires fabric 0.17', suggested_action: null, filename: null }],
      warnings: [],
      recommendations: [],
    },
  };
}

// ---------------------------------------------------------------------------
// Shared mock installer for UpdatesSection tests
// ---------------------------------------------------------------------------

async function updatesSectionMock(page: Page) {
  const updatesByInstance: Record<string, UpdateInfo[]> = {
    'vanilla-instance': VANILLA_UPDATES,
    // The locked instance has updates too — the point of the locked-instance
    // test is that Agora refuses to act on them, not that there are none.
    'locked-instance': VANILLA_UPDATES,
  };

  await page.addInitScript(
    (params: { updates: Record<string, UpdateInfo[]>; mods: typeof INSTALLED_MODS }) => {
      const { updates, mods: installedMods } = params;

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
          // Multi-session launch state is a list; the backend never returns null.
          if (command === 'query_launch_state') return Promise.resolve([]);
          if (command === 'list_instances') {
            return Promise.resolve([
              { instance_id: 'vanilla-instance', name: 'Vanilla', minecraft_version: '1.21', loader: 'fabric', loader_version: '0.16.0', is_modpack: false, is_locked: false, last_launched_at: null, jvm_memory_mb: 4096, jvm_gc: 'G1GC', jvm_custom_args: '', created_at: '2026-01-01T00:00:00Z' },
              { instance_id: 'locked-instance', name: 'Locked Modded', minecraft_version: '1.20.1', loader: 'fabric', loader_version: '0.15.11', is_modpack: false, is_locked: true, last_launched_at: null, jvm_memory_mb: 4096, jvm_gc: 'G1GC', jvm_custom_args: '', created_at: '2026-01-01T00:00:00Z' },
            ]);
          }
          if (command === 'get_instance_detail') {
            const instanceId = args.instanceId as string;
            const locked = instanceId === 'locked-instance';
            const row = {
              instance_id: instanceId,
              name: locked ? 'Locked Modded' : 'Vanilla',
              minecraft_version: '1.21', loader: 'fabric', loader_version: '0.16.0',
              is_modpack: false, is_locked: locked, last_launched_at: null,
              jvm_memory_mb: 4096, jvm_gc: 'G1GC', jvm_custom_args: '',
              created_at: '2026-01-01T00:00:00Z',
            };
            return Promise.resolve({
              row,
              manifest: {
                instance_id: instanceId, name: row.name, created_from_pack: null,
                minecraft_version: '1.21', loader: 'fabric', loader_version: '0.16.0',
                mods: installedMods.map((mod) => ({
                  filename: mod.filename, registry_id: null, modrinth_id: null,
                  mod_jar_id: mod.mod_jar_id, source: 'curated', version: mod.version,
                  sha256: mod.sha256, installed_at: '2026-07-01T00:00:00Z',
                  enabled: true, content_type: 'mod',
                })),
                resourcepacks: [], shaders: [], datapacks: [], worlds: [],
                user_preferences: {},
              },
            });
          }
          if (command === 'list_instance_content') {
            return Promise.resolve(installedMods.map((mod) => ({
              key: `mod:${mod.filename}:${mod.sha256}`,
              filename: mod.filename,
              display_name: mod.display_name,
              version: mod.version,
              content_type: 'mod',
              enabled: true,
              installed_at: '2026-07-01T00:00:00Z',
              source: 'curated',
              source_label: 'Agora',
              source_url: null,
              registry_id: mod.mod_jar_id,
              modrinth_id: null,
              mod_jar_id: mod.mod_jar_id,
              loader_mod_id: mod.mod_jar_id,
              size_bytes: 1234,
              file_present: true,
              resolved_path: `C:/instances/x/mods/${mod.filename}`,
              author: null,
              categories: ['Uncategorized'],
              icon_url: null,
              curation_status: 'curated',
              agora_score: null,
              modrinth_downloads: null,
              metadata_status: 'unavailable',
            })));
          }
          // No sweep has run, so the panel starts from an unchecked state.
          if (command === 'get_cached_instance_updates') return Promise.resolve([]);
          if (command === 'get_update_changelogs') return Promise.resolve([]);
          if (command === 'get_dependency_graph') return Promise.resolve(null);
          if (command === 'list_mod_groups') return Promise.resolve([]);
          if (command === 'batch_check_compat') return Promise.resolve({});
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
    { updates: updatesByInstance, mods: INSTALLED_MODS } as any,
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
  // The snapshot label moved under "Technical details"; the promise it carries
  // is stated in the body.
  await expect(page.getByText(/Agora saves a restore point/)).toBeVisible();
  await expect(page.getByRole('button', { name: /Apply Updates/ })).toBeVisible();
}

// ---------------------------------------------------------------------------
// Tests — updating installed content
//
// The old standalone "Updates" panel on the Instances tab is gone: the
// background sweep caches results and the instance editor's installed-content
// panel owns the update actions. These tests follow that surface — check,
// review the changelog, then the one canonical InstallFlow plan.
// ---------------------------------------------------------------------------

/** Open the instance editor for one instance and check for updates. */
async function checkUpdates(page: Page, instanceId: string) {
  await page.addInitScript((id) => {
    window.history.replaceState({ __agora: { type: 'instance-detail', instanceId: id } }, '');
  }, instanceId);
  await page.goto('/');
  await expect(page.getByRole('heading', { name: /Installed Mods/ })).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: 'Check for updates' }).click();
}

/** Check, then walk the changelog review that precedes the install plan. */
async function startUpdateAll(page: Page) {
  await checkUpdates(page, 'vanilla-instance');
  await page.getByRole('button', { name: 'Update all 2 mods' }).click();

  const changelog = page.getByRole('dialog');
  await expect(changelog.getByText('Review 2 updates')).toBeVisible();
  await changelog.getByRole('button', { name: 'Update all 2' }).click();
}

test.describe('Release C4 — updating installed content', () => {

  test('checking shows compatible updates for the unlocked instance', async ({ page }) => {
    await updatesSectionMock(page);
    await checkUpdates(page, 'vanilla-instance');

    // Both installed mods report an update, and one action covers them.
    await expect(page.getByRole('button', { name: 'Update all 2 mods' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Update Sodium to 0.6.0' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Update Lithium to 0.12.1' })).toBeVisible();
  });

  test('locked instance has no usable update action', async ({ page }) => {
    await updatesSectionMock(page);
    await checkUpdates(page, 'locked-instance');

    // The updates exist, but a locked instance must not act on them.
    await expect(page.getByRole('button', { name: 'Update all 2 mods' })).toBeDisabled();
  });

  test('update all reviews the changelog before opening the canonical InstallFlow', async ({ page }) => {
    await updatesSectionMock(page);
    await startUpdateAll(page);

    await expect.poll(() => totalInstallCalls(page)).toBeGreaterThanOrEqual(1);
    const resolveIdx = await lastInstallCall(page, 'resolve_install_plan');
    await resolveInstallCall(page, resolveIdx, makeBatchPlan());

    await expectReviewView(page);
  });

  test('batch intent contains every selected exact target version', async ({ page }) => {
    await updatesSectionMock(page);
    await startUpdateAll(page);

    const resolveIdx = await lastInstallCall(page, 'resolve_install_plan');
    const args = await page.evaluate(
      (idx) => (window as any).__installCalls[idx]?.args,
      resolveIdx,
    );

    expect(args.intent.action.type).toBe('batch-update');
    const items = args.intent.action.items as { itemId: string; targetVersion: string }[];
    expect(items).toHaveLength(2);
    expect(items.find((i) => i.itemId === 'sodium')?.targetVersion).toBe('0.6.0');
    expect(items.find((i) => i.itemId === 'lithium')?.targetVersion).toBe('0.12.1');

    // Resolve the plan to clean up the dialog.
    await resolveInstallCall(page, resolveIdx, makeBatchPlan());
    await expectReviewView(page);
  });

  test('failed artifact outcome leaves recovery messaging', async ({ page }) => {
    await updatesSectionMock(page);
    await startUpdateAll(page);

    const resolveIdx = await lastInstallCall(page, 'resolve_install_plan');
    await resolveInstallCall(page, resolveIdx, makeBatchPlan());

    await expect(page.getByRole('button', { name: /Apply Updates/ })).toBeVisible();
    await page.getByRole('button', { name: /Apply Updates/ }).click();

    await expect.poll(() => totalInstallCalls(page)).toBeGreaterThanOrEqual(2);
    const applyIdx = await lastInstallCall(page, 'apply_install_plan');
    await rejectInstallCall(page, applyIdx, new Error('Corrupt download: SHA-256 mismatch for sodium-0.6.0.jar'));

    // The editor runs the approved plan as a background task, so the failure
    // is reported there — with the backend's message, not a generic one.
    await expect(page.getByText('Installation failed', { exact: true }).first()).toBeVisible();
    await expect(page.getByText('Corrupt download: SHA-256 mismatch for sodium-0.6.0.jar').first()).toBeVisible();
  });

  test('successful batch reports completion', async ({ page }) => {
    await updatesSectionMock(page);
    await startUpdateAll(page);

    const resolveIdx = await lastInstallCall(page, 'resolve_install_plan');
    await resolveInstallCall(page, resolveIdx, makeBatchPlan());

    await expect(page.getByRole('button', { name: /Apply Updates/ })).toBeVisible();
    await page.getByRole('button', { name: /Apply Updates/ }).click();

    await expect.poll(() => totalInstallCalls(page)).toBeGreaterThanOrEqual(2);
    const applyIdx = await lastInstallCall(page, 'apply_install_plan');
    await resolveInstallCall(page, applyIdx, makeSuccessOutcome());

    await expect(page.getByText('Installation complete', { exact: true }).first()).toBeVisible();
    await expect(page.getByText('Installation completed successfully.').first()).toBeVisible();
  });

  test('health-blocked batch keeps the install and offers a rollback', async ({ page }) => {
    await updatesSectionMock(page);
    await startUpdateAll(page);

    const resolveIdx = await lastInstallCall(page, 'resolve_install_plan');
    await resolveInstallCall(page, resolveIdx, makeBatchPlan());

    await expect(page.getByRole('button', { name: /Apply Updates/ })).toBeVisible();
    await page.getByRole('button', { name: /Apply Updates/ }).click();

    await expect.poll(() => totalInstallCalls(page)).toBeGreaterThanOrEqual(2);
    const applyIdx = await lastInstallCall(page, 'apply_install_plan');
    await resolveInstallCall(page, applyIdx, makeHealthRollbackOutcome());

    // The files stay on disk so the user can look, and the recovery snapshot
    // stays one click away.
    await expect(page.getByText(/Health check found 1 blocker/).first()).toBeVisible();
    await expect(page.getByRole('button', { name: 'Roll back' }).first()).toBeVisible();
    await expect(page.getByRole('button', { name: 'Keep & review' }).first()).toBeVisible();
  });
});
