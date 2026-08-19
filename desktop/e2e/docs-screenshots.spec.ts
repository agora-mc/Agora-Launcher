import { test, expect, type Page } from '@playwright/test';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// ---------------------------------------------------------------------------
// Deterministic documentation screenshots.
//
// Each state is reached through the actual React UI with a mocked
// __TAURI_INTERNALS__ layer (same approach as the other e2e specs). All data
// is fixed synthetic content: no account data, tokens, local paths, server
// addresses, or private pack names. A fixed 1280x800 viewport is used and CSS
// animations are neutralized so every frame is stable.
// ---------------------------------------------------------------------------

const SCREENSHOT_DIR = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../web/public/screenshots',
);

const VIEWPORT = { width: 1280, height: 800 };

async function preparePage(page: Page) {
  await page.setViewportSize(VIEWPORT);
  await page.addStyleTag({
    content:
      '*,*::before,*::after{animation-duration:0.001ms!important;transition-duration:0.001ms!important;transition-delay:0ms!important}',
  });
}

async function shoot(page: Page, name: string) {
  await page.screenshot({ path: path.join(SCREENSHOT_DIR, `${name}.png`) });
}

// ---------------------------------------------------------------------------
// Onboarding — welcome step
// ---------------------------------------------------------------------------

test('docs screenshot: onboarding welcome', async ({ page }) => {
  await preparePage(page);
  await page.addInitScript(() => {
    let pollResolve: ((value: unknown) => void) | null = null;
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
        if (command === 'get_setting') {
          if (args.key === 'onboarding_complete') return Promise.resolve(false);
          if (args.key === 'modrinth_enabled') return Promise.resolve(true);
          if (args.key === 'ai_mcp_enabled') return Promise.resolve(false);
          if (args.key === 'ai_chat_enabled') return Promise.resolve(true);
          return Promise.resolve(null);
        }
        if (command === 'set_setting') return Promise.resolve(null);
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command.startsWith('plugin:event|') || command.startsWith('plugin:shell|')) return Promise.resolve(null);
        if (command === 'ensure_java_runtime') {
          return Promise.resolve({ path: '/mock/java21', version: 21, version_string: 'Java 21.0.1', source: 'Managed', arch: 'x64' });
        }
        if (command === 'github_login') {
          return Promise.resolve({
            device_code: 'device',
            user_code: 'ABCD-EFGH',
            verification_uri: 'https://github.com/login/device',
            expires_in: 900,
            interval: 1,
          });
        }
        if (command === 'github_login_poll') {
          return new Promise((resolve) => { pollResolve = resolve; });
        }
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __resolveGithubPoll(value: unknown) { pollResolve?.(value); },
    });
  });

  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Get Started' })).toBeVisible({ timeout: 15000 });
  await page.waitForTimeout(300);
  await shoot(page, 'onboarding-welcome');
});

// ---------------------------------------------------------------------------
// Creating an instance — Create Custom Instance dialog
// ---------------------------------------------------------------------------

test('docs screenshot: create instance', async ({ page }) => {
  await preparePage(page);
  await page.addInitScript(() => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
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
        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'launch_mode') return Promise.resolve('direct');
          if (key === 'modrinth_enabled') return Promise.resolve(true);
          if (key === 'last_home_visit') return Promise.resolve(null);
          return Promise.resolve(false);
        }
        if (command === 'set_setting') return Promise.resolve(null);
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

        if (command === 'list_manifest_loaders') {
          return Promise.resolve(['fabric', 'forge', 'quilt']);
        }
        if (command === 'list_manifest_mc_versions') {
          if (args.loader === 'forge') return Promise.resolve(['1.20.1']);
          return Promise.resolve(['1.21', '1.20.1']);
        }
        if (command === 'list_loader_versions') {
          const loader = args.loader as string;
          const mcVersion = args.mcVersion as string;
          if (loader === 'fabric' && mcVersion === '1.21') {
            return Promise.resolve([
              { loader: 'fabric', mc_version: '1.21', loader_version: '0.16.9', file_type: 'stable' },
              { loader: 'fabric', mc_version: '1.21', loader_version: '0.15.11', file_type: 'stable' },
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
        if (command === 'check_instance_health') {
          return Promise.resolve({ score: 'green', blockers: [], warnings: [] });
        }
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
      __tauriEventListeners: eventListeners,
      __callbacks: callbacks,
    });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await expect(page.getByText('No instances yet.')).toBeVisible();
  await page.getByRole('button', { name: '+ Create Instance' }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByText('Create Custom Instance')).toBeVisible();

  const nameInput = page.getByPlaceholder('Optimized Survival');
  await nameInput.fill('Optimized Survival');
  await expect(page.locator('select').nth(0)).toHaveValue('1.21', { timeout: 5000 });
  await expect(page.locator('select').nth(2)).toHaveValue('0.16.9');
  await expect(page.getByText('Agora will estimate memory from enabled pack content and system headroom.')).toBeVisible();

  await page.getByText('Set up a new isolated modpack profile with a verified modloader.').click();
  await page.waitForTimeout(300);
  await shoot(page, 'create-instance');
});

// ---------------------------------------------------------------------------
// Reviewing an install plan — Review Instance Changes dialog
// ---------------------------------------------------------------------------

interface InstallCall {
  command: string;
  args: Record<string, unknown>;
  resolve: (value: unknown) => void;
  reject: (reason?: unknown) => void;
}

function docsPlan() {
  const artifact = {
    type: 'download',
    itemId: 'test-mod',
    versionId: '1.0.0',
    source: { type: 'download', url: 'https://github.com/agora-mc/test-mod/releases/download/v1.0.0/test-mod-1.0.0.jar' },
    hashes: { values: [{ algorithm: 'sha256', value: 'a'.repeat(64) }] },
    size: 250_000,
    filename: 'test-mod-1.0.0.jar',
    metadata: { sourceType: 'curated', registryId: 'test-mod', modrinthId: null, contentType: 'mod', version: '1.0.0' },
  };
  return {
    fingerprint: 'plan-fp-docs-001',
    intent: {
      action: { type: 'install', sourceType: 'curated', itemId: 'test-mod', candidateVersion: '1.0.0' },
      targetInstance: 'test-instance',
      optionalDeps: { type: 'prompt' },
      requestedBy: 'interactive',
      overrides: { allowReplace: false, skipHealthScan: false, forceConflictResolution: {} },
    },
    operation: { type: 'install', artifact },
    dependencies: [
      {
        modJarId: 'fabric-api',
        requirement: 'required',
        source: 'jar',
        displayName: 'Fabric API',
        disposition: {
          type: 'install-candidate',
          artifact: {
            type: 'download',
            itemId: 'fabric-api',
            versionId: '0.99.0',
            source: { type: 'download', url: 'https://github.com/agora-mc/fabric-api/releases/download/v0.99.0/fabric-api-0.99.0.jar' },
            hashes: { values: [{ algorithm: 'sha256', value: 'b'.repeat(64) }] },
            size: 1_200_000,
            filename: 'fabric-api-0.99.0.jar',
            metadata: { sourceType: 'curated', registryId: 'fabric-api', modrinthId: null, contentType: 'mod', version: '0.99.0' },
          },
        },
      },
      {
        modJarId: 'cloth-config',
        requirement: 'optional',
        source: 'jar',
        displayName: 'Cloth Config',
        disposition: {
          type: 'install-candidate',
          artifact: {
            type: 'download',
            itemId: 'cloth-config',
            versionId: '15.0.0',
            source: { type: 'download', url: 'https://github.com/agora-mc/cloth-config/releases/download/v15.0.0/cloth-config-15.0.0.jar' },
            hashes: { values: [{ algorithm: 'sha256', value: 'c'.repeat(64) }] },
            size: 850_000,
            filename: 'cloth-config-15.0.0.jar',
            metadata: { sourceType: 'curated', registryId: 'cloth-config', modrinthId: null, contentType: 'mod', version: '15.0.0' },
          },
        },
      },
    ],
    conflicts: [],
    filesToAdd: [{ targetFilename: 'test-mod-1.0.0.jar', stagingFilename: 'staging-test-mod.jar', artifact, hashes: artifact.hashes, size: 250_000 }],
    filesToRemove: [],
    filesToDisable: [],
    snapshot: { label: 'Before installing test-mod 1.0.0', estimatedBytes: 500_000 },
    diskEstimate: { downloadBytes: 250_000, snapshotBytes: 500_000, applyOverheadBytes: 100_000, peakAdditionalBytes: 600_000, postCommitDeltaBytes: 250_000 },
    warnings: [
      {
        code: 'mod-imperfection',
        message: 'test-mod declares a malformed loader version range ("21.1.+") for fabric; it is treated as 21.1. Consider updating the mod.',
      },
    ],
    blockingErrors: [],
    pendingChoices: [],
    createdAt: '2026-07-12T17:00:00Z',
    instanceStateHash: 'abc123def456',
    registryRevision: 'v20260712',
  };
}

const CURATED_MOD: Record<string, unknown> = {
  id: 'test-mod',
  name: 'Test Mod',
  content_type: 'mod',
  download_strategy: 'github_release',
  source_identifier: 'test-mod/releases',
  sha256: '',
  upvotes: 10,
  downvotes: 2,
  net_score: 8,
  velocity: 1.5,
  status: 'active',
  is_immune: false,
  immunity_reason: null,
  allow_comments: true,
  icon_url: null,
  gallery_urls_json: null,
  date_added: '2026-01-01',
  compatible_versions_json: JSON.stringify([{ mc_version: '1.20.1', loader: 'fabric', mod_version: '1.0.0' }]),
  description: 'A test mod for verifying the install flow.',
  body_markdown: null,
  page_url: 'https://example.com/test-mod',
  license_id: 'MIT',
  source_updated_at: '2026-06-01T00:00:00Z',
  modrinth_id: null,
};

async function installFlowMock(page: Page) {
  await page.addInitScript((items: Record<string, unknown>) => {
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
        if (command === 'resolve_install_plan') {
          return new Promise((resolve, reject) => installCalls.push({ command, args, resolve, reject }));
        }
        if (command === 'apply_install_plan') {
          return new Promise((resolve, reject) => installCalls.push({ command, args, resolve, reject }));
        }
        if (command === 'cancel_install') return Promise.resolve(null);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);

        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'modrinth_enabled') return Promise.resolve(true);
          if (key === 'ai_chat_enabled') return Promise.resolve(false);
          if (key === 'mojang_launcher_path') return Promise.resolve('');
          if (key === 'launch_mode') return Promise.resolve('delegation');
          return Promise.resolve(null);
        }

        if (command === 'get_registry_status') {
          return Promise.resolve({ has_cached_db: true, cached_tag: 'test', cached_schema_version: 5, latest_tag: 'test', update_available: false, checked: true, message: 'Registry ready.' });
        }
        if (command === 'list_categories') return Promise.resolve([]);
        if (command === 'list_manifest_loaders') return Promise.resolve(['fabric', 'forge', 'quilt']);
        if (command === 'list_manifest_mc_versions') return Promise.resolve(['1.20.1', '1.21']);

        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'get_auth_status') return Promise.resolve(true);
        if (command === 'get_github_profile') return Promise.resolve(null);
        if (command === 'get_flag_rate_limit') return Promise.resolve(null);
        if (command === 'list_mod_reviews') return Promise.resolve([]);
        if (command === 'get_curated_annotation') return Promise.resolve(null);

        if (command === 'list_instances') {
          return Promise.resolve([
            { instance_id: 'test-instance', name: 'Test Instance', minecraft_version: '1.20.1', loader: 'fabric', loader_version: '0.15.11', is_modpack: false, is_locked: false, last_launched_at: null, jvm_memory_mb: 4096, jvm_gc: 'G1GC', jvm_custom_args: '', created_at: '2026-01-01T00:00:00Z' },
          ]);
        }
        if (command === 'check_instance_updates') return Promise.resolve([]);
        if (command === 'batch_check_compat') return Promise.resolve({});
        if (command === 'get_instance_detail') {
          const instanceId = args.instanceId as string;
          return Promise.resolve({
            row: { instance_id: instanceId, name: 'Test Instance', minecraft_version: '1.20.1', loader: 'fabric', loader_version: '0.15.11', is_modpack: false, is_locked: false, last_launched_at: null, jvm_memory_mb: 4096, jvm_gc: 'G1GC', jvm_custom_args: '', created_at: '2026-01-01T00:00:00Z' },
            manifest: { instance_id: instanceId, name: 'Test Instance', created_from_pack: null, minecraft_version: '1.20.1', loader: 'fabric', loader_version: '0.15.11', mods: [{ filename: 'installed-test-mod.jar', registry_id: null, modrinth_id: null, mod_jar_id: 'test-mod', source: 'manual_drag_drop', version: '1.0.0', sha256: 'a'.repeat(64), installed_at: '2026-07-01T00:00:00Z', enabled: true, content_type: 'mod' }], resourcepacks: [], shaders: [], datapacks: [], worlds: [], user_preferences: {} },
          });
        }
        if (command === 'list_instance_content') {
          return Promise.resolve([{
            key: 'mod:installed-test-mod.jar:' + 'a'.repeat(64),
            filename: 'installed-test-mod.jar',
            display_name: 'Test Mod',
            version: '1.0.0',
            content_type: 'mod',
            enabled: true,
            installed_at: '2026-07-01T00:00:00Z',
            source: 'manual_drag_drop',
            source_label: 'Manual',
            source_url: null,
            registry_id: null,
            modrinth_id: null,
            mod_jar_id: 'test-mod',
            loader_mod_id: 'test-mod',
            size_bytes: 1234,
            file_present: true,
            resolved_path: 'C:/instances/test-instance/mods/installed-test-mod.jar',
            author: null,
            categories: ['Uncategorized'],
            icon_url: null,
            curation_status: 'unknown',
            agora_score: null,
            modrinth_downloads: null,
            metadata_status: 'unavailable',
          }]);
        }
        if (command === 'list_snapshots') return Promise.resolve([]);
        if (command === 'list_loadout_profiles') return Promise.resolve([]);
        if (command === 'restore_snapshot') return Promise.resolve(null);

        if (command === 'get_registry_item') {
          return Promise.resolve((items as any)[args.itemId as string] ?? null);
        }
        if (command === 'fetch_modrinth_project') return Promise.resolve(null);
        if (command === 'is_modrinth_enabled') return Promise.resolve(true);
        if (command === 'list_mod_versions') {
          return Promise.resolve({
            items: [
              { version: '1.0.0', filename: 'test-mod-1.0.0.jar', mc_version: '1.20.1', loader: 'fabric', version_compat: 'compatible', release_date: '2026-06-01', sha256: 'abc123def456' },
              { version: '0.9.0', filename: 'test-mod-0.9.0.jar', mc_version: '1.20.1', loader: 'fabric', version_compat: 'major_match', release_date: '2026-05-01', sha256: 'def789abc012' },
            ],
            hasMore: false,
          });
        }
        if (command === 'list_mod_versions_load_more') {
          return Promise.resolve({ items: [], hasMore: false });
        }
        if (command === 'list_raw_modrinth_versions') return Promise.resolve([]);

        if (command === 'browse_search') {
          return Promise.resolve({
            items: [
              { id: 'test-mod', source: 'curated', registryItem: { id: 'test-mod', name: 'Test Mod', content_type: 'mod', download_strategy: 'github_release', upvotes: 10, downvotes: 2, net_score: 8, velocity: 1.5 }, modrinthResult: null, name: 'Test Mod', iconUrl: null, description: 'A test mod for verifying the install flow.', contentType: 'mod', heroImageUrl: null, author: null, categories: [], downloads: null, follows: null, upvotes: 10, downvotes: 2, netScore: 8, supportedVersions: [], sourcePageUrl: null },
            ],
            total: 1,
            page: 0,
            hasMore: false,
          });
        }
        if (command === 'browse_load_more') return Promise.resolve({ items: [], total: 0, page: 1, hasMore: false });
        if (command === 'for_you_items') return Promise.resolve({ items: [] });

        if (command === 'browse_items') {
          return Promise.resolve([
            { id: 'test-mod', name: 'Test Mod', content_type: 'mod', download_strategy: 'github_release', source_identifier: 'test-mod', upvotes: 10, downvotes: 2, net_score: 8, velocity: 1.5, description: 'A test mod', icon_url: null },
          ]);
        }

        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __installCalls: installCalls,
    });
  }, { 'test-mod': CURATED_MOD });
}

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

async function pickFirstInstanceSelect(page: Page, value: string) {
  await page.locator('select').first().selectOption(value);
}

test('docs screenshot: install plan review', async ({ page }) => {
  await preparePage(page);
  await installFlowMock(page);
  await page.goto('/');

  await page.getByRole('button', { name: 'Browse', exact: true }).click();
  await expect(page.getByRole('button', { name: 'View Details', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'View Details', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Test Mod', exact: true })).toBeVisible();

  await page.getByRole('button', { name: 'Install to Instance' }).click();
  await pickFirstInstanceSelect(page, 'test-instance');
  await page.getByRole('button', { name: 'Next: Choose Version' }).click();
  await page.getByText('test-mod-1.0.0.jar').click();
  await page.getByRole('button', { name: /(Install|Replace with) test-mod-1.0.0.jar/ }).click();

  await expect.poll(() => totalInstallCalls(page)).toBeGreaterThanOrEqual(1);
  const resolveIdx = await lastInstallCall(page, 'resolve_install_plan');
  await resolveInstallCall(page, resolveIdx, docsPlan());

  await expect(page.getByText('Review Instance Changes')).toBeVisible({ timeout: 10000 });
  await expect(page.getByText('+1 to add')).toBeVisible();
  await expect(page.getByText('Fabric API')).toBeVisible();
  await page.waitForTimeout(300);
  await shoot(page, 'install-plan-review');
});

// ---------------------------------------------------------------------------
// Loader compatibility repair — Health Check dialog
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

test('docs screenshot: loader compatibility repair', async ({ page }) => {
  await preparePage(page);
  await page.addInitScript((fixture: Record<string, unknown>) => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const eventListeners = new Map<string, unknown>();
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
        if (command === 'get_setting') {
          if (args.key === 'onboarding_complete') return Promise.resolve(true);
          if (args.key === 'launch_mode') return Promise.resolve('direct');
          return Promise.resolve(false);
        }
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'plugin:event|listen') {
          eventListeners.set(args.event as string, args.handler as unknown);
          return Promise.resolve(1);
        }
        if (command === 'plugin:event|unlisten') return Promise.resolve(1);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command === 'list_instances') return Promise.resolve([row]);
        if (command === 'get_instance_detail') {
          return Promise.resolve({
            row,
            manifest: {
              instance_id: row.instance_id,
              name: row.name,
              minecraft_version: row.minecraft_version,
              loader: row.loader,
              loader_version: row.loader_version,
              mods: [], resourcepacks: [], shaders: [], datapacks: [], worlds: [],
              user_preferences: {},
            },
            snapshot_readiness: 'ready',
            snapshot_error: null,
          });
        }
        if (command === 'check_instance_crash') return Promise.resolve(null);
        if (command === 'check_instance_health') {
          return Promise.resolve({
            score: 'red',
            blockers: [{
              kind: 'loader_version_mismatch', mod_id: null,
              filename: null,
              message: 'The installed fabric loader 0.16 does not satisfy the loader requirements of enabled mods.',
              suggested_action: 'Switch the instance loader to fabric 0.19.0.',
              loader_compatibility: fixture,
            }],
            warnings: [],
            recommendations: [],
            scan_token: 'health-e2e-scan',
          });
        }
        if (command === 'check_all_instance_health') return Promise.resolve([]);
        if (command === 'launch_instance_with_recovery') return Promise.resolve(4242);
        if (command === 'launch_instance_direct') return Promise.resolve(4242);
        if (command === 'launch_instance') return Promise.resolve(null);
        if (command === 'disable_mod_for_test') return Promise.resolve(null);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __tauriEventListeners: eventListeners,
      __callbacks: callbacks,
    });
  }, loaderIssueFixture);

  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await expect(page.getByRole('button', { name: 'Launch' }).first()).toBeVisible({ timeout: 10000 });
  await page.getByRole('button', { name: 'Launch' }).first().click();

  await expect(page.getByText('Loader: fabric 0.16')).toBeVisible({ timeout: 10000 });
  await expect(page.getByText('Recommended version: 0.19.0')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Switch and launch' })).toBeVisible();
  await page.waitForTimeout(300);
  await shoot(page, 'loader-compatibility-repair');
});

// ---------------------------------------------------------------------------
// Crash Doctor — guided isolation dialog
// ---------------------------------------------------------------------------

test('docs screenshot: crash doctor', async ({ page }) => {
  await preparePage(page);
  await page.addInitScript(() => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const eventListeners = new Map<string, number>();

    const SUSPECT_1 = { mod_id: 'suspect-mod', filename: 'suspect-mod.jar', total_score: 0.85, breakdown: { stack_frame_score: 0.65, curated_conflict_score: 0.2 }, is_dependent_of: null };
    const SUSPECT_2 = { mod_id: 'second-suspect', filename: 'second-suspect.jar', total_score: 0.45, breakdown: { stack_frame_score: 0.3, prior_local_crashes: 0.15 }, is_dependent_of: null };

    const initialResult = {
      fingerprint: { exception_class: 'java.lang.NullPointerException', top_frames: ['test'] },
      signature_name: 'NullPointerException in rendering',
      suspects: [SUSPECT_1, SUSPECT_2],
      suggested_action: { kind: 'GuidedDisable', next_suspect: SUSPECT_1 },
      ruled_out: [],
    };

    const evidenceResult = {
      evidence: {
        sources: [{
          meta: {
            basename: 'latest.log', kind: 'LatestLog', size_bytes: 120,
            truncated: false, stale: false, supplementary: false,
            modified_at: '2026-07-12T18:00:00Z', line_count: 3,
          },
          text: 'Mock crash log:\njava.lang.NullPointerException\n\tat net.minecraft.class_123',
        }],
        primary_index: 0,
        aggregate_bytes: 120,
        any_truncated: false,
        any_stale: false,
        failure_category: 'CrashReport',
      },
      fingerprint: initialResult.fingerprint,
      triage: {
        matched: true,
        signature_name: initialResult.signature_name,
        solution_markdown: 'Disable the identified rendering mod and test again.',
        action_button_json: null,
      },
      suspects: initialResult.suspects,
      failure_category: 'CrashReport',
    };

    const instanceRow = { instance_id: 'crash-test-instance', name: 'Crash Test', loader: 'fabric', loader_version: '0.16', minecraft_version: '1.21', is_locked: false, last_launched_at: null };

    const internals = {
      transformCallback(cb: (...args: unknown[]) => void) { const id = ++callbackId; callbacks.set(id, cb); return id; },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'launch_mode') return Promise.resolve('direct');
          if (key === 'modrinth_enabled') return Promise.resolve(true);
          if (key === 'last_home_visit') return Promise.resolve(null);
          return Promise.resolve(false);
        }
        if (command === 'set_setting') return Promise.resolve(null);
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'get_registry_status') return Promise.resolve({ has_cached_db: true, cached_tag: 'test', cached_schema_version: 5, latest_tag: 'test', update_available: false, checked: true, message: 'Registry ready.' });
        if (command === 'check_registry_update') return Promise.resolve(null);
        if (command === 'list_categories') return Promise.resolve([]);
        if (command === 'list_manifest_loaders') return Promise.resolve([]);
        if (command === 'list_manifest_mc_versions') return Promise.resolve([]);
        if (command === 'for_you_items') return Promise.resolve([]);
        if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
        if (command === 'query_launch_state') return Promise.resolve(null);

        if (command === 'plugin:event|listen') { eventListeners.set(args.event as string, args.handler as number); return Promise.resolve(1); }
        if (command === 'plugin:event|unlisten') return Promise.resolve(1);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command.startsWith('plugin:sql|')) return Promise.resolve(null);

        if (command === 'list_instances') return Promise.resolve([instanceRow]);
        if (command === 'check_instance_crash') return Promise.resolve(null);
        if (command === 'check_instance_updates') return Promise.resolve([]);
        if (command === 'get_lkg_marker') return Promise.resolve(null);
        if (command === 'list_snapshots') return Promise.resolve([]);
        if (command === 'detect_drift') return Promise.resolve({ added: [], removed: [], modified: [] });

        if (command === 'create_snapshot') {
          return Promise.resolve({ id: 'snap-recovery-001', label: (args.label as string) ?? 'Recovery snapshot', created_at: '2026-07-12T18:00:00Z', file_count: 42, size_estimate: 2_500_000, is_lkg: false, is_current_lkg: false, is_pre_restore: false });
        }
        if (command === 'restore_snapshot') return Promise.resolve(null);

        if (command === 'investigate_manual') return Promise.resolve(initialResult);
        if (command === 'investigate_crash') return Promise.resolve(initialResult);
        if (command === 'investigate_instance_evidence') return Promise.resolve(evidenceResult);
        if (command === 'pick_and_investigate_crash_evidence') return Promise.resolve(evidenceResult);
        if (command === 'read_crash_log') return Promise.resolve('Mock crash log:\njava.lang.NullPointerException\n\tat net.minecraft.class_123');

        if (command === 'get_disable_plan') return Promise.resolve({ dependents: [] });
        if (command === 'disable_mod_for_test') return Promise.resolve(null);
        if (command === 'enable_mod_for_test') return Promise.resolve(null);

        if (command === 'confirm_crash_fix') return Promise.resolve(null);
        if (command === 'report_still_crashing') {
          return Promise.resolve({
            fingerprint: { exception_class: 'java.lang.RuntimeException', top_frames: ['test'] },
            signature_name: 'RuntimeException in physics',
            suspects: [SUSPECT_2],
            suggested_action: { kind: 'GuidedDisable', next_suspect: SUSPECT_2 },
            ruled_out: ['suspect-mod'],
          });
        }

        if (command === 'explain_crash') {
          return Promise.resolve('This crash is likely caused by suspect-mod.jar conflicting with the rendering pipeline.');
        }

        if (command === 'check_instance_health') {
          return Promise.resolve({
            score: 'green',
            blockers: [],
            warnings: [],
            recommendations: [],
            scan_token: 'crash-e2e-scan',
          });
        }
        if (command === 'launch_instance_direct') return Promise.resolve(4242);
        if (command === 'launch_instance') return Promise.resolve(null);

        return Promise.resolve(null);
      },
    };

    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __tauriEventListeners: eventListeners,
      __callbacks: callbacks,
    });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'My Instances' }).click();
  await expect(page.getByRole('button', { name: 'Troubleshoot' })).toBeVisible({ timeout: 10000 });
  await page.getByRole('button', { name: 'Troubleshoot' }).click();
  await expect(page.getByRole('heading', { name: 'Crash Doctor' })).toBeVisible({ timeout: 10000 });
  await expect(page.getByText('SUSPECTS')).toBeVisible({ timeout: 10000 });
  await expect(page.getByText('0.85')).toBeVisible();
  await page.waitForTimeout(300);
  await shoot(page, 'crash-doctor');
});

// ---------------------------------------------------------------------------
// Privacy / Lockdown — Settings > Privacy with Lockdown Mode enabled
// ---------------------------------------------------------------------------

test('docs screenshot: privacy lockdown', async ({ page }) => {
  await preparePage(page);
  await page.addInitScript(() => {
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
        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'modrinth_enabled') return Promise.resolve(true);
          if (key === 'browse_curated_only') return Promise.resolve(false);
          if (key === 'ai_mcp_enabled') return Promise.resolve(false);
          if (key === 'ai_chat_enabled') return Promise.resolve(true);
          if (key === 'mojang_launcher_path') return Promise.resolve('');
          if (key === 'java_path') return Promise.resolve(null);
          if (key === 'java_runtime_mode') return Promise.resolve('automatic');
          if (key === 'always_pre_touch') return Promise.resolve(true);
          if (key === 'install_auto_confirm_clean') return Promise.resolve(false);
          if (key === 'install_always_auto_confirm') return Promise.resolve(false);
          if (key === 'launch_mode') return Promise.resolve('delegation');
          if (key === 'advanced_mode') return Promise.resolve('true');
          if (key === 'health_preferences') return Promise.resolve(null);
          if (key === 'network_modrinth_enabled') return Promise.resolve(true);
          if (key === 'network_modrinth_cdn_enabled') return Promise.resolve(true);
          if (key === 'network_registry_sync_enabled') return Promise.resolve(true);
          if (key === 'network_github_oauth_enabled') return Promise.resolve(true);
          if (key === 'network_mojang_metadata_enabled') return Promise.resolve(true);
          if (key === 'network_mojang_content_enabled') return Promise.resolve(true);
          if (key === 'network_loader_enabled') return Promise.resolve(true);
          if (key === 'network_msa_enabled') return Promise.resolve(true);
          if (key === 'network_adoptium_enabled') return Promise.resolve(true);
          if (key === 'network_lockdown_enabled') return Promise.resolve(false);
          return Promise.resolve(null);
        }
        if (command === 'set_setting') return Promise.resolve(null);
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'msa_get_status') return Promise.resolve(null);
        if (command === 'copilot_status') return Promise.resolve(null);
        if (command === 'get_auth_status') return Promise.resolve(null);
        if (command === 'get_github_profile') return Promise.resolve(null);
        if (command === 'get_mcp_status') return Promise.resolve(null);
        if (command === 'list_instances') return Promise.resolve([]);
        if (command === 'list_snapshots') return Promise.resolve([]);
        if (command === 'check_all_instance_health') return Promise.resolve([]);
        if (command === 'get_registry_status') {
          return Promise.resolve({ has_cached_db: true, cached_tag: 'test', cached_schema_version: 5, latest_tag: 'test', update_available: false, checked: true, message: 'Registry ready.' });
        }
        if (command === 'check_registry_update') return Promise.resolve(null);
        if (command === 'list_categories') return Promise.resolve([]);
        if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
        if (command === 'for_you_items') return Promise.resolve([]);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const sections = page.getByRole('navigation', { name: 'Settings sections' });
  await expect(sections.getByRole('tab', { name: 'Privacy' })).toBeVisible({ timeout: 10000 });
  await sections.getByRole('tab', { name: 'Privacy' }).click();

  await expect(page.getByText('Privacy & Transparency')).toBeVisible({ timeout: 10000 });
  await expect(page.getByRole('switch', { name: 'Lockdown Mode' })).toBeVisible();
  await page.getByRole('switch', { name: 'Lockdown Mode' }).click();
  await expect(page.getByRole('switch', { name: 'Lockdown Mode' })).toBeChecked();
  await expect(page.locator('#settings-privacy')).toBeInViewport();
  await page.waitForTimeout(300);
  await shoot(page, 'privacy-lockdown');
});
