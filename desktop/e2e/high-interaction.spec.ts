import { test, expect, type Page } from '@playwright/test';

// ---------------------------------------------------------------------------
// DEEPSEEK-7: High Interaction (real operations) end-to-end verification.
// SOL-2 §20 authorizes the approved seams (remove via InstallFlow, health
// inspection, loader, snapshot compare, Crash Doctor navigation). These specs
// exercise the read surface with REAL mock-backed data and prove two of the
// approved bridges open their Standard surfaces end-to-end. The remaining
// bridges (loader, snapshot compare, Crash Doctor) are covered by the
// operationBridges + host unit/integration suites and Luna's visual regression.
// ---------------------------------------------------------------------------

interface InstallCall {
  command: string;
  args: Record<string, unknown>;
  resolve: (value: unknown) => void;
  reject: (reason?: unknown) => void;
}

function makeRemovePlan() {
  return {
    fingerprint: 'plan-remove-001',
    intent: {
      action: { type: 'remove', filename: 'example.jar' },
      targetInstance: 'hi-instance',
      optionalDeps: { type: 'prompt' },
      requestedBy: 'interactive',
      overrides: { allowReplace: false, skipHealthScan: false, forceConflictResolution: {} },
    },
    operation: { type: 'remove', targetFilename: 'example.jar', reverseDependents: [] },
    dependencies: [],
    conflicts: [],
    filesToAdd: [],
    filesToRemove: [{ filename: 'example.jar', content_type: 'mod', reason: 'requested removal' }],
    filesToDisable: [],
    snapshot: { label: 'Before removing example.jar', estimatedBytes: 100_000 },
    diskEstimate: { downloadBytes: 0, snapshotBytes: 100_000, applyOverheadBytes: 0, peakAdditionalBytes: 100_000, postCommitDeltaBytes: -250_000 },
    warnings: [],
    blockingErrors: [],
    pendingChoices: [],
    createdAt: '2026-08-10T00:00:00Z',
    instanceStateHash: 'abc123def456',
    registryRevision: 'v20260810',
  };
}

async function installHighInteractionMock(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const installCalls: InstallCall[] = [];
    const row = {
      instance_id: 'hi-instance',
      name: 'HI Test',
      loader: 'fabric',
      loader_version: '0.16',
      minecraft_version: '1.21',
      is_locked: false,
      last_launched_at: null,
      jvm_memory_mb: 2048,
      jvm_gc: 'G1GC',
      jvm_custom_args: '',
      created_at: '2026-01-01T00:00:00Z',
    };
    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'resolve_install_plan' || command === 'apply_install_plan') {
          return new Promise((resolve, reject) => installCalls.push({ command, args, resolve, reject } as any));
        }
        if (command === 'cancel_install') return Promise.resolve(null);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command === 'get_setting') {
          if (args.key === 'onboarding_complete') return Promise.resolve(true);
          if (args.key === 'launch_mode') return Promise.resolve('delegation');
          return Promise.resolve(false);
        }
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'list_instances') return Promise.resolve([row]);
        if (command === 'get_instance_detail') {
          return Promise.resolve({
            row,
            manifest: {
              instance_id: row.instance_id,
              name: row.name,
              created_from_pack: null,
              minecraft_version: row.minecraft_version,
              loader: row.loader,
              loader_version: row.loader_version,
              is_locked: false,
              mods: [{
                filename: 'example.jar',
                registry_id: 'example',
                modrinth_id: null,
                source: 'registry',
                version: '1.0.0',
                sha256: 'a'.repeat(64),
                installed_at: '2026-07-01T00:00:00Z',
                enabled: true,
                content_type: 'mod',
              }],
              resourcepacks: [],
              shaders: [],
              datapacks: [],
              worlds: [],
              user_preferences: {},
            },
            snapshot_readiness: 'ready',
            snapshot_error: null,
          });
        }
        if (command === 'query_launch_state') return Promise.resolve(null);
        if (command === 'check_instance_health') {
          return Promise.resolve({
            score: 'yellow',
            blockers: [],
            warnings: [{
              kind: 'unknown_mod',
              mod_id: 'example',
              filename: 'example.jar',
              message: 'Example warning',
              suggested_action: null,
            }],
            recommendations: [],
            scan_token: 'hi-scan',
          });
        }
        if (command === 'list_snapshots') return Promise.resolve([]);
        if (command === 'investigate_instance_evidence') return Promise.resolve(null);
        if (command === 'recommend_instance_memory') {
          return Promise.resolve({
            recommended_mb: 4096,
            tier_label: '4 GB',
            tier_index: 1,
            is_large_resource_pack_adjustment: false,
            ram_capped: false,
            insufficient_system_ram: false,
            system_ram_mb: 16384,
            next_tier_mb: 8192,
            next_tier_label: '8 GB',
            factors: [],
            explanation: 'more ram',
          });
        }
        if (command === 'list_java_runtimes') {
          return Promise.resolve([{ path: '/jdk', version: 17, version_string: '17', source: 'managed', arch: 'amd64' }]);
        }
        if (command === 'list_instance_content') return Promise.resolve([]);
        if (command === 'enrich_instance_content') return Promise.resolve([]);
        if (command === 'check_instance_crash') return Promise.resolve(null);
        if (command === 'list_crash_reports') return Promise.resolve([]);
        if (command === 'list_loadout_profiles') return Promise.resolve([]);
        if (command === 'restore_snapshot') return Promise.resolve(null);
        if (command === 'detect_drift') {
          return Promise.resolve({ entries: [], created_at: '2026-08-10T00:00:00Z' });
        }
        if (command === 'check_instance_updates') return Promise.resolve([]);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __installCalls: installCalls,
    });
  });
}

async function resolveInstall(page: Page, index: number, result: unknown) {
  await page.evaluate(
    ({ idx, res }: { idx: number; res: unknown }) => {
      const calls = (window as any).__installCalls as InstallCall[];
      if (calls[idx]) calls[idx].resolve(res);
    },
    { idx: index, res: result },
  );
}

async function lastInstallCall(page: Page, command: string): Promise<number> {
  let index = -1;
  await expect.poll(async () => {
    const calls: InstallCall[] = await page.evaluate(() => (window as any).__installCalls ?? []);
    const indices = calls
      .map((c, i) => ({ c, i }))
      .filter(({ c }) => c.command === command)
      .map(({ i }) => i);
    index = indices.length > 0 ? indices[indices.length - 1] : -1;
    return index;
  }).toBeGreaterThanOrEqual(0);
  return index;
}

async function openHighInteraction(page: Page) {
  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await page.getByRole('button', { name: 'Edit' }).first().click();
  await page.getByRole('button', { name: 'High Interaction view' }).click();
}

test('High Interaction renders a real instance and approved health review opens the Standard dialog', async ({ page }) => {
  await installHighInteractionMock(page);
  await openHighInteraction(page);

  // Live scene renders with real instance data.
  await expect(page.getByText('HI Test').first()).toBeVisible();
  await expect(page.getByText('High Interaction').first()).toBeVisible();
  await expect(page.getByText('Example warning').first()).toBeVisible();

  // Approved health inspection: Review health opens the Standard reviewOnly
  // HealthDialog and leaves High Interaction (option (a) terminal lifecycle).
  await page.getByRole('button', { name: 'Review health' }).click();
  await expect(page.getByRole('heading', { name: 'Health Check' })).toBeVisible();
  await expect(page.getByText('Example warning').first()).toBeVisible();
});

test('approved remove through InstallFlow: Stage remove re-resolves and opens the canonical InstallFlow', async ({ page }) => {
  await installHighInteractionMock(page);
  await openHighInteraction(page);

  // The installed mod renders with the approved Stage removal action.
  await expect(page.getByRole('button', { name: 'Stage removal' })).toBeVisible();
  await page.getByRole('button', { name: 'Stage removal' }).click();

  // The bridge re-resolves per route and opens the canonical InstallFlow,
  // which resolves a remove plan.
  const resolveIdx = await lastInstallCall(page, 'resolve_install_plan');
  await resolveInstall(page, resolveIdx, makeRemovePlan());

  // InstallFlow review view appears with the planned removal.
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByText('Review Instance Changes')).toBeVisible();
  await expect(page.getByText('example.jar')).toBeVisible();
  await expect(page.getByText(/Before removing/)).toBeVisible();
});
