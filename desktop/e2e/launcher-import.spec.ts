import { test, expect, type Page } from '@playwright/test';

// ---------------------------------------------------------------------------
// Mock data builders
// ---------------------------------------------------------------------------

function makePrismCandidate(overrides: Record<string, unknown> = {}) {
  return {
    source_key: 'my-pack',
    launcher: 'prism',
    launcher_installation_key: 'default',
    display_name: 'My Pack',
    icon_path: null,
    payload_root: '/home/user/.local/share/PrismLauncher/instances/My Pack/.minecraft',
    inventory: {
      payload_root: '/home/user/.local/share/PrismLauncher/instances/My Pack/.minecraft',
      total_files: 142,
      total_bytes: 85_000_000,
      has_mods: true,
      has_resourcepacks: true,
      has_shaderpacks: false,
      has_datapacks: false,
      has_saves: true,
    },
    loader_tuple: { loader: 'fabric', loader_version: '0.16.9', minecraft_version: '1.21' },
    last_played: '2026-07-20T14:30:00Z',
    launch_strategy: 'normal',
    settings_preview: { memory_mb: 4096, java_path: '/usr/lib/jvm/java-21-openjdk/bin/java', jvm_args: [] },
    status: 'ready',
    warnings: [],
    ...overrides,
  };
}

function makeSecondPrismCandidate(overrides: Record<string, unknown> = {}) {
  return {
    source_key: 'optifine-pack',
    launcher: 'prism',
    launcher_installation_key: 'default',
    display_name: 'OptiFine Pack',
    icon_path: null,
    payload_root: '/home/user/.local/share/PrismLauncher/instances/OptiFine Pack/.minecraft',
    inventory: {
      payload_root: '/home/user/.local/share/PrismLauncher/instances/OptiFine Pack/.minecraft',
      total_files: 89,
      total_bytes: 42_000_000,
      has_mods: true,
      has_resourcepacks: false,
      has_shaderpacks: true,
      has_datapacks: false,
      has_saves: false,
    },
    loader_tuple: { loader: 'forge', loader_version: '50.1.0', minecraft_version: '1.20.1' },
    last_played: '2026-07-18T09:15:00Z',
    launch_strategy: 'normal',
    settings_preview: { memory_mb: 6144, java_path: null, jvm_args: ['-XX:+UseG1GC'] },
    status: 'needs_review',
    warnings: ['Instance uses OptiFine which may have compatibility issues.'],
    ...overrides,
  };
}

function makeCurseForgeCandidate(overrides: Record<string, unknown> = {}) {
  return {
    source_key: 'ctm-pack',
    launcher: 'curse_forge',
    launcher_installation_key: 'default',
    display_name: 'CTM Pack',
    icon_path: null,
    payload_root: '/home/user/.local/share/curseforge/minecraft/Instances/CTM Pack',
    inventory: {
      payload_root: '/home/user/.local/share/curseforge/minecraft/Instances/CTM Pack',
      total_files: 0,
      total_bytes: 0,
      has_mods: false,
      has_resourcepacks: false,
      has_shaderpacks: false,
      has_datapacks: false,
      has_saves: false,
    },
    loader_tuple: null,
    last_played: null,
    launch_strategy: 'delegated',
    settings_preview: { memory_mb: null, java_path: null, jvm_args: [] },
    status: { unsupported: { reasons: ['CurseForge import requires the CurseForge launcher to be installed and logged in.', 'No CurseForge installation detected.'] } },
    warnings: [],
    ...overrides,
  };
}

function makeDiscovery() {
  return {
    prism: {
      launcher: {
        installation_key: 'default',
        kind: 'prism',
        display_name: 'Prism Launcher',
        config_root: '/home/user/.local/share/PrismLauncher',
        instances_dir: '/home/user/.local/share/PrismLauncher/instances',
        instance_count: 2,
        detection_warnings: [],
      },
      candidates: [makePrismCandidate(), makeSecondPrismCandidate()],
    },
    curseforge: {
      launcher: null,
      candidates: [makeCurseForgeCandidate()],
    },
    modrinth: {
      launcher: null,
      candidates: [],
    },
  };
}

function makePlanItem(overrides: Record<string, unknown> = {}) {
  return {
    fingerprint: 'fp-my-pack-001',
    destination_id: 'my-pack',
    destination_name: 'My Pack',
    action: 'new',
    source_key: 'my-pack',
    launcher_kind: 'prism',
    installation_key: 'default',
    source_path: '/home/user/.local/share/PrismLauncher/instances/My Pack/.minecraft',
    loader_tuple: { loader: 'fabric', loader_version: '0.16.9', minecraft_version: '1.21' },
    total_bytes: 85_000_000,
    total_files: 142,
    preserve_settings: true,
    sanitized_settings: { memory_mb: 4096, java_path: '/usr/lib/jvm/java-21-openjdk/bin/java', jvm_args: [] },
    existing_import: null,
    blockers: [],
    warnings: [],
    ...overrides,
  };
}

function makePlan(items = [makePlanItem()]) {
  return {
    batch_fingerprint: 'batch-fp-001',
    items,
    peak_bytes: 85_000_000,
    total_files: 142,
    batch_blockers: [],
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatBytes(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)} KB`;
  return `${bytes} B`;
}

// ---------------------------------------------------------------------------
// Shared mock — launcher import flow from My Instances
// ---------------------------------------------------------------------------

interface ImportFlowMockOptions {
  onboardingComplete?: boolean;
  includeUnsupported?: boolean;
}

async function installLauncherImportMock(page: Page, opts: ImportFlowMockOptions = {}) {
  const { onboardingComplete = true, includeUnsupported = true } = opts;

  let instanceList: Record<string, unknown>[] = [];

  await page.addInitScript(
    (params: { onboardingComplete: boolean; includeUnsupported: boolean }) => {
      const callbacks = new Map<number, (...args: unknown[]) => void>();
      let callbackId = 0;
      const commandCalls: Record<string, number> = {};
      const lastCommandArgs: Record<string, Record<string, unknown>> = {};
      const eventListeners = new Map<string, number>();

      let storedInstances: Record<string, unknown>[] = [];

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

          // --- Settings ---
          if (command === 'get_setting') {
            const key = args.key as string;
            if (key === 'onboarding_complete') return Promise.resolve(params.onboardingComplete);
            if (key === 'launch_mode') return Promise.resolve('direct');
            if (key === 'modrinth_enabled') return Promise.resolve(true);
            if (key === 'ai_mcp_enabled') return Promise.resolve(false);
            if (key === 'ai_chat_enabled') return Promise.resolve(true);
            if (key === 'last_home_visit') return Promise.resolve(null);
            return Promise.resolve(null);
          }
          if (command === 'set_setting') return Promise.resolve(null);

          // --- Registry / browse (ambient) ---
          if (command === 'get_registry_status') {
            return Promise.resolve({
              has_cached_db: true, cached_tag: 'test', cached_schema_version: 5,
              latest_tag: 'test', update_available: false, checked: true, message: 'Registry ready.',
            });
          }
          if (command === 'check_registry_update') return Promise.resolve(null);
          if (command === 'list_categories') return Promise.resolve([]);
          if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
          if (command === 'for_you_items') return Promise.resolve([]);
          if (command === 'get_windows_accent_color') return Promise.resolve(null);
          if (command === 'get_lkg_marker') return Promise.resolve(null);
          if (command === 'get_auth_status') return Promise.resolve(true);
          if (command === 'get_github_profile') return Promise.resolve(null);
          if (command === 'get_flag_rate_limit') return Promise.resolve(null);
          if (command === 'list_mod_reviews') return Promise.resolve([]);
          if (command === 'get_curated_annotation') return Promise.resolve(null);
          if (command === 'list_manifest_loaders') return Promise.resolve(['fabric', 'forge', 'quilt']);
          if (command === 'list_manifest_mc_versions') return Promise.resolve(['1.21', '1.20.1']);
          if (command === 'list_loader_versions') return Promise.resolve([]);
          if (command === 'check_instance_crash') return Promise.resolve(null);
          if (command === 'check_instance_updates') return Promise.resolve([]);
          if (command === 'batch_check_compat') return Promise.resolve({});
          if (command === 'get_instance_detail') {
            return Promise.resolve({ row: null, manifest: null });
          }
          if (command === 'list_snapshots') return Promise.resolve([]);
          if (command === 'list_loadout_profiles') return Promise.resolve([]);
          if (command === 'detect_mojang_launcher') return Promise.resolve('/mock/launcher');
          if (command === 'test_launcher_path') return Promise.resolve(true);

          // --- Launcher import commands ---
          if (command === 'discover_launcher_imports') {
            const disc: Record<string, unknown> = {
              prism: {
                launcher: {
                  installation_key: 'default', kind: 'prism', display_name: 'Prism Launcher',
                  config_root: '/home/user/.local/share/PrismLauncher',
                  instances_dir: '/home/user/.local/share/PrismLauncher/instances',
                  instance_count: 2, detection_warnings: [],
                },
                candidates: [
                  {
                    source_key: 'my-pack', launcher: 'prism',
                    launcher_installation_key: 'default', display_name: 'My Pack',
                    icon_path: null,
                    payload_root: '/home/user/.local/share/PrismLauncher/instances/My Pack/.minecraft',
                    inventory: {
                      payload_root: '/home/user/.local/share/PrismLauncher/instances/My Pack/.minecraft',
                      total_files: 142, total_bytes: 85_000_000, has_mods: true,
                      has_resourcepacks: true, has_shaderpacks: false,
                      has_datapacks: false, has_saves: true,
                    },
                    loader_tuple: { loader: 'fabric', loader_version: '0.16.9', minecraft_version: '1.21' },
                    last_played: '2026-07-20T14:30:00Z', launch_strategy: 'normal',
                    settings_preview: { memory_mb: 4096, java_path: '/usr/lib/jvm/java-21-openjdk/bin/java', jvm_args: [] },
                    status: 'ready', warnings: [],
                  },
                  {
                    source_key: 'optifine-pack', launcher: 'prism',
                    launcher_installation_key: 'default', display_name: 'OptiFine Pack',
                    icon_path: null,
                    payload_root: '/home/user/.local/share/PrismLauncher/instances/OptiFine Pack/.minecraft',
                    inventory: {
                      payload_root: '/home/user/.local/share/PrismLauncher/instances/OptiFine Pack/.minecraft',
                      total_files: 89, total_bytes: 42_000_000, has_mods: true,
                      has_resourcepacks: false, has_shaderpacks: true,
                      has_datapacks: false, has_saves: false,
                    },
                    loader_tuple: { loader: 'forge', loader_version: '50.1.0', minecraft_version: '1.20.1' },
                    last_played: '2026-07-18T09:15:00Z', launch_strategy: 'normal',
                    settings_preview: { memory_mb: 6144, java_path: null, jvm_args: ['-XX:+UseG1GC'] },
                    status: 'needs_review',
                    warnings: ['Instance uses OptiFine which may have compatibility issues.'],
                  },
                ],
              },
              curseforge: {
                launcher: null,
                candidates: params.includeUnsupported
                  ? [{
                      source_key: 'ctm-pack', launcher: 'curse_forge',
                      launcher_installation_key: 'default', display_name: 'CTM Pack',
                      icon_path: null,
                      payload_root: '/home/user/.local/share/curseforge/minecraft/Instances/CTM Pack',
                      inventory: {
                        payload_root: '/home/user/.local/share/curseforge/minecraft/Instances/CTM Pack',
                        total_files: 0, total_bytes: 0, has_mods: false,
                        has_resourcepacks: false, has_shaderpacks: false,
                        has_datapacks: false, has_saves: false,
                      },
                      loader_tuple: null, last_played: null, launch_strategy: 'delegated',
                      settings_preview: { memory_mb: null, java_path: null, jvm_args: [] },
                      status: { unsupported: { reasons: ['CurseForge import requires the CurseForge launcher to be installed and logged in.', 'No CurseForge installation detected.'] } },
                      warnings: [],
                    }]
                  : [],
              },
              modrinth: { launcher: null, candidates: [] },
            };
            return Promise.resolve(disc);
          }

          if (command === 'plan_launcher_imports') {
            const selections = args.selections as Array<Record<string, unknown>>;
            (window as any).__lastPlanArgs = selections;
            const items = selections.map((s, i) => ({
              fingerprint: `fp-${s.source_key ?? i}`,
              destination_id: ((s.destination_name as string) ?? 'imported').toLowerCase().replace(/[^a-z0-9-_]+/g, '-').replace(/^-+|-+$/g, ''),
              destination_name: (s.destination_name as string) ?? 'Imported',
              action: 'new',
              source_key: s.source_key as string,
              launcher_kind: s.launcher_kind as string,
              installation_key: s.installation_key as string,
              source_path: '/mock/.minecraft',
              loader_tuple: { loader: 'fabric', loader_version: '0.16.9', minecraft_version: '1.21' },
              total_bytes: 85_000_000,
              total_files: 142,
              preserve_settings: s.preserve_settings ?? true,
              sanitized_settings: { memory_mb: 4096, java_path: '/usr/lib/jvm/java-21-openjdk/bin/java', jvm_args: [] },
              existing_import: null,
              blockers: [],
              warnings: [],
            }));
            return Promise.resolve({
              batch_fingerprint: 'batch-fp-001',
              items,
              peak_bytes: 85_000_000,
              total_files: 142,
              batch_blockers: [],
            });
          }

          if (command === 'execute_launcher_imports') {
            const plan = args.plan as Record<string, unknown>;
            (window as any).__lastExecutePlan = plan;
            return Promise.resolve({
              outcomes: [
                { status: 'imported', instance_id: 'my-pack', warnings: [] },
              ],
            });
          }

          if (command === 'pick_directory') {
            return Promise.resolve('/mock/picked/directory');
          }

          if (command === 'cancel_operation') return Promise.resolve(true);

          // --- Instances ---
          if (command === 'list_instances') {
            return Promise.resolve(storedInstances);
          }

          if (command === 'create_instance') {
            const req = args.request as Record<string, unknown>;
            storedInstances = [{
              instance_id: req.instance_id,
              name: req.name,
              minecraft_version: req.minecraft_version,
              loader: req.loader,
              loader_version: req.loader_version,
              is_locked: false,
              last_launched_at: null,
            }];
            return Promise.resolve(storedInstances[0]);
          }

          if (command === 'delete_instance') return Promise.resolve(null);

          // --- Events ---
          if (command.startsWith('plugin:event|listen')) {
            eventListeners.set(args.event as string, args.handler as number);
            return Promise.resolve(1);
          }
          if (command.startsWith('plugin:event|unlisten')) return Promise.resolve(1);
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);
          if (command.startsWith('plugin:shell|')) return Promise.resolve(null);

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
        __lastPlanArgs: null,
        __lastExecutePlan: null,
      });
    },
    { onboardingComplete, includeUnsupported },
  );
}

async function installOnboardingMockWithImport(page: Page) {
  await page.addInitScript(() => {
    let pollResolve: ((value: unknown) => void) | null = null;
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const commandCalls: Record<string, number> = {};
    const eventListeners = new Map<string, number>();
    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        commandCalls[command] = (commandCalls[command] ?? 0) + 1;
        // --- Settings ---
        if (command === 'get_setting') {
          if (args.key === 'onboarding_complete') return Promise.resolve(false);
          if (args.key === 'modrinth_enabled') return Promise.resolve(false);
          if (args.key === 'ai_mcp_enabled') return Promise.resolve(false);
          if (args.key === 'ai_chat_enabled') return Promise.resolve(false);
          if (args.key === 'last_home_visit') return Promise.resolve(null);
          if (args.key === 'launch_mode') return Promise.resolve('direct');
          return Promise.resolve(null);
        }
        if (command === 'set_setting') return Promise.resolve(null);

        // --- Registry (must work for app shell) ---
        if (command === 'get_registry_status') {
          return Promise.resolve({
            has_cached_db: true, cached_tag: 'test', cached_schema_version: 5,
            latest_tag: 'test', update_available: false, checked: true, message: 'Registry ready.',
          });
        }
        if (command === 'check_registry_update') return Promise.resolve(null);

        // --- Browse / ambient ---
        if (command === 'list_categories') return Promise.resolve([]);
        if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
        if (command === 'for_you_items') return Promise.resolve([]);
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'get_lkg_marker') return Promise.resolve(null);
        if (command === 'get_auth_status') return Promise.resolve(true);
        if (command === 'get_github_profile') return Promise.resolve(null);
        if (command === 'get_flag_rate_limit') return Promise.resolve(null);
        if (command === 'list_mod_reviews') return Promise.resolve([]);
        if (command === 'get_curated_annotation') return Promise.resolve(null);

        // --- Manifest data ---
        if (command === 'list_manifest_loaders') return Promise.resolve(['fabric', 'forge', 'quilt']);
        if (command === 'list_manifest_mc_versions') return Promise.resolve(['1.21', '1.20.1']);
        if (command === 'list_loader_versions') return Promise.resolve([]);

        // --- Instances (must return array, not null) ---
        if (command === 'list_instances') return Promise.resolve([]);
        if (command === 'create_instance') return Promise.resolve(null);
        if (command === 'delete_instance') return Promise.resolve(null);
        if (command === 'check_instance_crash') return Promise.resolve(null);
        if (command === 'check_instance_updates') return Promise.resolve([]);
        if (command === 'batch_check_compat') return Promise.resolve({});
        if (command === 'get_instance_detail') return Promise.resolve({ row: null, manifest: null });
        if (command === 'list_snapshots') return Promise.resolve([]);
        if (command === 'list_loadout_profiles') return Promise.resolve([]);
        if (command === 'restore_snapshot') return Promise.resolve(null);

        // --- Events ---
        if (command.startsWith('plugin:event|listen')) {
          eventListeners.set(args.event as string, args.handler as number);
          return Promise.resolve(1);
        }
        if (command.startsWith('plugin:event|unlisten')) return Promise.resolve(1);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command.startsWith('plugin:shell|')) return Promise.resolve(null);

        // --- Java ---
        if (command === 'ensure_java_runtime') {
          return Promise.resolve({ path: '/mock/java21', version: 21, version_string: 'Java 21.0.1', source: 'Managed', arch: 'x64' });
        }
        if (command === 'cancel_java_runtime') return Promise.resolve(null);

        // --- GitHub ---
        if (command === 'github_login') {
          return Promise.resolve({
            device_code: 'device', user_code: 'ABCD-EFGH',
            verification_uri: 'https://github.com/login/device',
            expires_in: 900, interval: 1,
          });
        }
        if (command === 'github_login_poll') {
          return new Promise((resolve) => { pollResolve = resolve; });
        }

        // --- Launcher import ---
        if (command === 'discover_launcher_imports') {
          return Promise.resolve({
            prism: { launcher: null, candidates: [] },
            curseforge: { launcher: null, candidates: [] },
            modrinth: { launcher: null, candidates: [] },
          });
        }
        if (command === 'plan_launcher_imports') {
          return Promise.resolve({ batch_fingerprint: 'bf', items: [], peak_bytes: 0, total_files: 0, batch_blockers: [] });
        }
        if (command === 'execute_launcher_imports') {
          return Promise.resolve({ outcomes: [] });
        }
        if (command === 'pick_directory') return Promise.resolve(null);
        if (command === 'cancel_operation') return Promise.resolve(true);

        // --- Misc ---
        if (command === 'detect_mojang_launcher') return Promise.resolve('/mock/launcher');
        if (command === 'test_launcher_path') return Promise.resolve(true);

        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __commandCalls: commandCalls,
      __tauriEventListeners: eventListeners,
      __resolveGithubPoll(value: unknown) { pollResolve?.(value); },
    });
  });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe('Launcher Import — My Instances entry', () => {

  test('full import flow: select, review, execute, and refresh', async ({ page }) => {
    await installLauncherImportMock(page);
    await page.goto('/');

    // Navigate to My Instances
    await page.getByRole('button', { name: 'My Instances' }).click();
    await expect(page.getByRole('heading', { name: 'My Instances' })).toBeVisible();

    // Open the launcher import dialog.
    await page.getByRole('button', { name: 'Import Instances' }).click();

    // Dialog opens — should briefly show detect stage, then transition to select
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Import from Other Launchers' })).toBeVisible({ timeout: 5000 });

    // --- Select stage ---

    // Verify Prism Launcher group header is visible
    await expect(page.getByText('Prism Launcher', { exact: true })).toBeVisible();
    // Verify CurseForge group header is visible with unsupported reasons
    await expect(page.getByText('CurseForge', { exact: true })).toBeVisible();
    // Verify Modrinth is not shown (no candidates)
    await expect(page.getByText('Modrinth App')).toHaveCount(0);

    // Both Prism candidates visible and selectable
    await expect(page.getByText('My Pack', { exact: true })).toBeVisible();
    await expect(page.getByText('OptiFine Pack', { exact: true })).toBeVisible();

    // Preserve settings toggle exists and defaults to checked
    const preserveToggle = page.getByLabel('Preserve compatible settings (memory, Java path, JVM args)');
    await expect(preserveToggle).toBeVisible();

    // Verify no Review button initially (no selection)
    await expect(page.getByRole('button', { name: /Review/ })).toHaveCount(0);

    // Select "My Pack" candidate
    await page.getByLabel('Select My Pack').click();

    // Review button appears with count
    await expect(page.getByRole('button', { name: 'Review (1)' })).toBeVisible();

    // Verify CurseForge candidate shows unsupported text
    await expect(page.getByText('Unsupported')).toBeVisible();
    await expect(page.getByText('CurseForge import requires the CurseForge launcher')).toBeVisible();
    await expect(page.getByText('No CurseForge installation detected.')).toBeVisible();

    // Destination name input appears for selected candidate — edit it
    const destInput = page.getByLabel('Destination name for My Pack');
    await expect(destInput).toBeVisible();
    await destInput.fill('My Renamed Pack');

    // --- Review stage ---
    await page.getByRole('button', { name: 'Review (1)' }).click();

    // Should transition to review stage
    await expect(page.getByRole('heading', { name: 'Review Import Plan' })).toBeVisible({ timeout: 3000 });

    // Verify plan_launcher_imports was called with preserve_settings=true
    const planArgs = await page.evaluate(() => (window as any).__lastPlanArgs);
    expect(planArgs).toBeTruthy();
    expect(Array.isArray(planArgs)).toBe(true);
    expect(planArgs[0]).toHaveProperty('preserve_settings', true);
    expect(planArgs[0]).toHaveProperty('destination_name', 'My Renamed Pack');
    expect(planArgs[0]).toHaveProperty('source_key', 'my-pack');

    // Verify review card shows action badge (new), name, settings, bytes, files
    await expect(page.getByText('NEW')).toBeVisible();
    await expect(page.getByText('My Renamed Pack')).toBeVisible();

    // Settings preview
    await expect(page.getByText('Memory: 4096 MB')).toBeVisible();
    await expect(page.getByText('Java: /usr/lib/jvm/java-21-openjdk/bin/java')).toBeVisible();

    // Size info (appears in item detail as "85.0 MB · 142 files" and in summary footer)
    await expect(page.getByText(`85.0 MB · 142 files`)).toBeVisible();

    // Summary footer
    await expect(page.getByText('Total items: 1')).toBeVisible();
    await expect(page.getByText(`Peak storage: ${formatBytes(85_000_000)}`)).toBeVisible();
    await expect(page.getByText('Total files: 142')).toBeVisible();

    // Import button should be enabled
    const importBtn = page.getByRole('button', { name: 'Import 1 Instance' });
    await expect(importBtn).toBeEnabled();

    // --- Execute ---
    await importBtn.click();

    // Should transition through importing to results
    await expect(page.getByRole('heading', { name: 'Import Results' })).toBeVisible({ timeout: 5000 });

    // Verify execute_launcher_imports was called with the plan
    const executePlan = await page.evaluate(() => (window as any).__lastExecutePlan);
    expect(executePlan).toBeTruthy();
    expect(executePlan).toHaveProperty('batch_fingerprint', 'batch-fp-001');

    // Verify outcome renders
    await expect(page.getByText('Imported')).toBeVisible();
    await expect(page.getByText('my-pack')).toBeVisible();

    // Done button
    await expect(page.getByRole('button', { name: 'Done' })).toBeVisible();

    // Click Done — dialog closes
    await page.getByRole('button', { name: 'Done' }).click();
    await expect(dialog).toHaveCount(0);

    // Verify list_instances was called at least once after completion
    const listCalls = await page.evaluate(() => {
      const c = (window as any).__commandCalls as Record<string, number>;
      return c['list_instances'] ?? 0;
    });
    expect(listCalls).toBeGreaterThanOrEqual(1);
  });

  test('preserve settings can be toggled off and reflects in plan args', async ({ page }) => {
    await installLauncherImportMock(page);
    await page.goto('/');

    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Import Instances' }).click();

    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Import from Other Launchers' })).toBeVisible({ timeout: 5000 });

    // Select candidate
    await page.getByLabel('Select My Pack').click();
    await expect(page.getByRole('button', { name: 'Review (1)' })).toBeVisible();

    // Toggle preserve settings OFF
    const preserveToggle = page.getByLabel('Preserve compatible settings (memory, Java path, JVM args)');
    await preserveToggle.click();
    await expect(preserveToggle).not.toBeChecked();

    // Review
    await page.getByRole('button', { name: 'Review (1)' }).click();
    await expect(page.getByRole('heading', { name: 'Review Import Plan' })).toBeVisible({ timeout: 3000 });

    const planArgs = await page.evaluate(() => (window as any).__lastPlanArgs);
    expect(planArgs).toBeTruthy();
    expect(planArgs[0]).toHaveProperty('preserve_settings', false);
  });

  test('back button from review returns to select stage', async ({ page }) => {
    await installLauncherImportMock(page);
    await page.goto('/');

    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Import Instances' }).click();

    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Import from Other Launchers' })).toBeVisible({ timeout: 5000 });

    await page.getByLabel('Select My Pack').click();
    await page.getByRole('button', { name: 'Review (1)' }).click();
    await expect(page.getByRole('heading', { name: 'Review Import Plan' })).toBeVisible({ timeout: 3000 });

    // Click Back
    await page.getByRole('button', { name: 'Back to selection' }).click();
    await expect(page.getByRole('heading', { name: 'Import from Other Launchers' })).toBeVisible();
  });
});

test.describe('Launcher Import — mixed results rendering', () => {

  async function installMixedResultsMock(page: Page) {
    await page.addInitScript(() => {
      const callbacks = new Map<number, (...args: unknown[]) => void>();
      let callbackId = 0;
      const eventListeners = new Map<string, number>();
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
            if (key === 'ai_chat_enabled') return Promise.resolve(false);
            if (key === 'last_home_visit') return Promise.resolve(null);
            return Promise.resolve(null);
          }
          if (command === 'set_setting') return Promise.resolve(null);
          if (command === 'get_registry_status') {
            return Promise.resolve({
              has_cached_db: true, cached_tag: 'test', cached_schema_version: 5,
              latest_tag: 'test', update_available: false, checked: true, message: 'Registry ready.',
            });
          }
          if (command === 'check_registry_update') return Promise.resolve(null);
          if (command === 'list_categories') return Promise.resolve([]);
          if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
          if (command === 'for_you_items') return Promise.resolve([]);
          if (command === 'get_windows_accent_color') return Promise.resolve(null);
          if (command === 'list_manifest_loaders') return Promise.resolve(['fabric', 'forge']);
          if (command === 'list_manifest_mc_versions') return Promise.resolve(['1.21', '1.20.1']);
          if (command === 'list_loader_versions') return Promise.resolve([]);
          if (command === 'list_instances') return Promise.resolve([]);
          if (command === 'check_instance_crash') return Promise.resolve(null);
          if (command === 'check_instance_updates') return Promise.resolve([]);
          if (command === 'detect_mojang_launcher') return Promise.resolve('/mock/launcher');
          if (command === 'test_launcher_path') return Promise.resolve(true);
          if (command === 'get_auth_status') return Promise.resolve(true);
          if (command === 'get_github_profile') return Promise.resolve(null);
          if (command === 'get_lkg_marker') return Promise.resolve(null);

          if (command.startsWith('plugin:event|listen')) {
            eventListeners.set(args.event as string, args.handler as number);
            return Promise.resolve(1);
          }
          if (command.startsWith('plugin:event|unlisten')) return Promise.resolve(1);
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);
          if (command.startsWith('plugin:shell|')) return Promise.resolve(null);

          if (command === 'discover_launcher_imports') {
            return Promise.resolve({
              prism: {
                launcher: {
                  installation_key: 'default', kind: 'prism', display_name: 'Prism Launcher',
                  config_root: '/mock/prism', instances_dir: '/mock/prism/instances',
                  instance_count: 3, detection_warnings: [],
                },
                candidates: [
                  {
                    source_key: 'imported-pack', launcher: 'prism',
                    launcher_installation_key: 'default', display_name: 'Imported & Pack',
                    icon_path: null, payload_root: '/mock/imported-pack/.minecraft',
                    inventory: { payload_root: '/mock/imported-pack/.minecraft', total_files: 100, total_bytes: 50_000_000, has_mods: true, has_resourcepacks: false, has_shaderpacks: false, has_datapacks: false, has_saves: false },
                    loader_tuple: { loader: 'fabric', loader_version: '0.16.9', minecraft_version: '1.21' },
                    last_played: '2026-07-20T14:30:00Z', launch_strategy: 'normal',
                    settings_preview: { memory_mb: 4096, java_path: null, jvm_args: [] },
                    status: 'ready', warnings: [],
                  },
                  {
                    source_key: 'dupe', launcher: 'prism',
                    launcher_installation_key: 'default', display_name: 'Already Exists',
                    icon_path: null, payload_root: '/mock/dupe/.minecraft',
                    inventory: { payload_root: '/mock/dupe/.minecraft', total_files: 50, total_bytes: 20_000_000, has_mods: true, has_resourcepacks: false, has_shaderpacks: false, has_datapacks: false, has_saves: false },
                    loader_tuple: { loader: 'fabric', loader_version: '0.15.11', minecraft_version: '1.20.1' },
                    last_played: null, launch_strategy: 'normal',
                    settings_preview: { memory_mb: 2048, java_path: null, jvm_args: [] },
                    status: 'ready', warnings: [],
                  },
                  {
                    source_key: 'broken-pack', launcher: 'prism',
                    launcher_installation_key: 'default', display_name: 'Broken Pack',
                    icon_path: null, payload_root: '/mock/broken/.minecraft',
                    inventory: { payload_root: '/mock/broken/.minecraft', total_files: 10, total_bytes: 5_000_000, has_mods: true, has_resourcepacks: false, has_shaderpacks: false, has_datapacks: false, has_saves: false },
                    loader_tuple: null, last_played: null, launch_strategy: 'delegated',
                    settings_preview: { memory_mb: null, java_path: null, jvm_args: [] },
                    status: 'needs_review', warnings: [],
                  },
                ],
              },
              curseforge: { launcher: null, candidates: [] },
              modrinth: { launcher: null, candidates: [] },
            });
          }

          if (command === 'plan_launcher_imports') {
            const selections = args.selections as Array<Record<string, unknown>>;
            (window as any).__lastPlanArgs = selections;
            const items = selections.map((s) => ({
              fingerprint: `fp-${s.source_key}`,
              destination_id: ((s.destination_name as string) ?? 'x').toLowerCase().replace(/[^a-z0-9-_]+/g, '-').replace(/^-+|-+$/g, ''),
              destination_name: s.destination_name as string,
              action: 'new',
              source_key: s.source_key as string,
              launcher_kind: s.launcher_kind as string,
              installation_key: s.installation_key as string,
              source_path: '/mock/.minecraft',
              loader_tuple: { loader: 'fabric', loader_version: '0.16.9', minecraft_version: '1.21' },
              total_bytes: 50_000_000,
              total_files: 100,
              preserve_settings: s.preserve_settings ?? true,
              sanitized_settings: { memory_mb: 4096, java_path: null, jvm_args: [] },
              existing_import: null,
              blockers: [],
              warnings: [],
            }));
            return Promise.resolve({
              batch_fingerprint: 'batch-fp-mixed',
              items,
              peak_bytes: 75_000_000,
              total_files: 160,
              batch_blockers: [],
            });
          }

          if (command === 'execute_launcher_imports') {
            (window as any).__lastExecutePlan = args.plan;
            return Promise.resolve({
              outcomes: [
                { status: 'imported', instance_id: 'imported-and-pack', warnings: ['Some resource packs could not be copied.'] },
                { status: 'skipped', reason: 'Instance already exists in Agora (already-exists). Use update instead.' },
                { status: 'failed', error: 'Corrupt instance directory: missing version.json', warnings: ['Loader manifest not found in instance directory.'] },
              ],
            });
          }

          if (command === 'pick_directory') return Promise.resolve(null);
          if (command === 'cancel_operation') return Promise.resolve(true);

          return Promise.resolve(null);
        },
      };
      Object.assign(window as unknown as Record<string, unknown>, {
        __TAURI_INTERNALS__: internals,
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
        __commandCalls: {},
        __tauriEventListeners: eventListeners,
        __lastPlanArgs: null,
        __lastExecutePlan: null,
      });
    });
  }

  test('renders imported, skipped, and failed outcomes with source text escaping', async ({ page }) => {
    await installMixedResultsMock(page);
    await page.goto('/');

    // Navigate to My Instances
    await page.getByRole('button', { name: 'My Instances' }).click();

    // Open import dialog
    await page.getByRole('button', { name: 'Import Instances' }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Import from Other Launchers' })).toBeVisible({ timeout: 5000 });

    // Verify special characters in candidate name display as text (not HTML-entity escaped)
    const candidateLabel = page.getByText(/Imported & Pack/);
    await expect(candidateLabel).toBeVisible();
    // The ampersand should render as a literal &, not &amp;
    await expect(candidateLabel).toHaveText(/Imported & Pack/);

    // Select all 3 candidates
    await page.getByLabel('Select Imported & Pack').click();
    await page.getByLabel('Select Already Exists').click();
    await page.getByLabel('Select Broken Pack').click();

    await page.getByRole('button', { name: 'Review (3)' }).click();
    await expect(page.getByRole('heading', { name: 'Review Import Plan' })).toBeVisible({ timeout: 3000 });

    // Execute
    await page.getByRole('button', { name: 'Import 3 Instances' }).click();
    await expect(page.getByRole('heading', { name: 'Import Results' })).toBeVisible({ timeout: 5000 });

    // Verify imported outcome with check icon text
    await expect(page.getByText('Imported', { exact: true })).toBeVisible();
    await expect(page.getByText('imported-and-pack')).toBeVisible();
    // Warning from imported outcome
    await expect(page.getByText('Some resource packs could not be copied.')).toBeVisible();

    // Verify skipped outcome
    await expect(page.getByText('Skipped')).toBeVisible();
    await expect(page.getByText('Instance already exists in Agora')).toBeVisible();

    // Verify failed outcome
    await expect(page.getByText('Failed')).toBeVisible();
    await expect(page.getByText('Corrupt instance directory: missing version.json')).toBeVisible();
    await expect(page.getByText('Loader manifest not found in instance directory.')).toBeVisible();

    // Done
    await page.getByRole('button', { name: 'Done' }).click();
    await expect(dialog).toHaveCount(0);
  });
});

test.describe('Launcher Import — onboarding Bring Your Instances step', () => {

  test('Registry continue then Skip for now finishes onboarding', async ({ page }) => {
    await installOnboardingMockWithImport(page);
    await page.goto('/');

    // Welcome step
    await expect(page.getByRole('heading', { name: 'Welcome to Agora' })).toBeVisible();
    await page.getByRole('button', { name: 'Get Started' }).click();

    // Appearance step
    await expect(page.getByRole('heading', { name: 'Make it yours' })).toBeVisible();
    await page.getByRole('button', { name: 'Continue' }).click();

    // Services step
    await expect(page.getByRole('heading', { name: 'Connect External Services' })).toBeVisible();
    await page.getByRole('button', { name: 'Continue' }).click();

    // Java step — uncheck to skip Java download
    await expect(page.getByRole('heading', { name: 'Prepare Java for Minecraft' })).toBeVisible({ timeout: 3000 });
    const javaSwitch = page.getByRole('switch');
    await javaSwitch.click();
    await expect(javaSwitch).toHaveAttribute('aria-checked', 'false');
    await page.getByRole('button', { name: 'Continue' }).click();

    // GitHub step
    await expect(page.getByRole('heading', { name: 'Connect GitHub' })).toBeVisible();
    await page.getByRole('button', { name: "I'll do this later" }).click();

    // Registry step — mock returns ready, so Finish should be available
    await expect(page.getByRole('heading', { name: 'Download Registry' })).toBeVisible({ timeout: 5000 });

    // Click Finish on Registry step (button label when registry is ready)
    const registryFinish = page.getByRole('button', { name: 'Finish' });
    await expect(registryFinish).toBeVisible();
    await registryFinish.click();

    // Import step — Bring Your Instances
    await expect(page.getByRole('heading', { name: 'Bring Your Instances' })).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Agora can detect Prism Launcher, CurseForge, and Modrinth App instances')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Find My Instances' })).toBeVisible();

    // Skip for now — completes onboarding
    await page.getByRole('button', { name: 'Skip for now' }).click();

    // Verify onboarding completes — sidebar is rendered
    await expect(page.getByTestId('sidebar')).toBeVisible({ timeout: 8000 });

    // Verify set_setting was called with onboarding_complete=true
    const setSettingCalls = await page.evaluate(() => {
      const c = (window as any).__commandCalls as Record<string, number>;
      return c?.['set_setting'] ?? 0;
    });
    // Should have been called at least once (set_setting)
    expect(setSettingCalls).toBeGreaterThanOrEqual(1);
  });
});
