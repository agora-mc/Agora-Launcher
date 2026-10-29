import { test, expect, type Page } from '@playwright/test';

/**
 * Browse must agree with the Health Check about a listed mod whose file is
 * gone from mods/. The manifest says the mod is installed; the Health Check's
 * `missing_files` says the file is not there.
 */

const REGISTRY_ITEM = {
  id: 'test-mod',
  name: 'Test Mod',
  content_type: 'mod',
  download_strategy: 'github_release',
  source_identifier: 'test/mod',
  sha256: 'abc123',
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
  compatible_versions_json: null,
  description: 'A test mod.',
  body_markdown: null,
  page_url: null,
  license_id: 'MIT',
  source_updated_at: null,
  modrinth_id: null,
};

const INSTANCE = {
  instance_id: 'a',
  name: 'Test Instance',
  minecraft_version: '1.21',
  loader: 'fabric',
  loader_version: '0.16.9',
  is_locked: false,
  last_launched_at: null,
};

const ENTRY = {
  filename: 'test-mod.jar',
  registry_id: 'test-mod',
  modrinth_id: null,
  source: 'curated',
  sha256: 'a'.repeat(64),
  installed_at: '2026-07-01T00:00:00Z',
  version: '1.0.0',
  enabled: true,
  content_type: 'mod',
};

function report(missingFiles: string[]) {
  return {
    score: missingFiles.length > 0 ? 'yellow' : 'green',
    scan_token: 'scan',
    blockers: [],
    recommendations: [],
    warnings: missingFiles.length > 0
      ? [{
          kind: 'manifest_drift',
          mod_id: null,
          filename: null,
          message: `The instance manifest tracks ${missingFiles.length} enabled mod file(s) that are absent from mods/: ${missingFiles.join(', ')}.`,
          suggested_action: 'Repair or reinstall the modpack before treating dependent mods as installed.',
        }]
      : [],
    missing_files: missingFiles,
  };
}

async function boot(page: Page, missingFiles: string[]) {
  await page.setViewportSize({ width: 1024, height: 800 });
  await page.addInitScript(
    ({ instance, entry, item, healthReport }) => {
      localStorage.setItem('agora-ui-preferences', JSON.stringify({ version: 1, fontScale: 1 }));
      // Open the mod page with the instance as its install context.
      window.history.replaceState(
        { __agora: { type: 'mod-detail', itemId: 'test-mod', browseInstanceId: 'a' } },
        '',
      );
      const detail = {
        row: { ...instance, is_modpack: false, jvm_memory_mb: 4096, jvm_gc: 'G1GC', jvm_custom_args: '', created_at: '2026-06-01T00:00:00Z' },
        pack_gaps: [],
        snapshot_readiness: 'ready',
        manifest: {
          instance_id: 'a', name: 'Test Instance', created_from_pack: null, minecraft_version: '1.21', loader: 'fabric',
          loader_version: '0.16.9', is_locked: false, mods: [entry], resourcepacks: [], shaders: [], datapacks: [], worlds: [],
          user_preferences: {},
        },
      };
      Object.assign(window as unknown as Record<string, unknown>, {
        __TAURI_INTERNALS__: {
          transformCallback() { return 1; },
          unregisterCallback() {},
          invoke(command: string, args: Record<string, unknown> = {}) {
            if (command === 'get_setting') {
              if (args.key === 'onboarding_complete') return Promise.resolve(true);
              if (args.key === 'modrinth_enabled') return Promise.resolve(false);
              return Promise.resolve(null);
            }
            if (command === 'get_registry_status') return Promise.resolve({ has_cached_db: true, cached_tag: 't', cached_schema_version: 5, latest_tag: 't', update_available: false, checked: true, message: 'Catalog ready.' });
            if (command === 'get_auth_status') return Promise.resolve(false);
            if (command === 'get_registry_item') return Promise.resolve(args.itemId === item.id ? item : null);
            if (command === 'get_curated_annotation') return Promise.resolve(null);
            if (command === 'get_governance_config') return Promise.resolve(null);
            if (command === 'get_item_vote') return Promise.resolve({ vote: null, conflicted: false });
            if (command === 'list_instances') return Promise.resolve([instance]);
            if (command === 'get_instance_detail') return Promise.resolve(detail);
            if (command === 'check_instance_health') return Promise.resolve(healthReport);
            if (command.startsWith('plugin:event|')) return Promise.resolve(1);
            if (/^(list_|for_you|search_|query_launch_state)/.test(command)) return Promise.resolve([]);
            return Promise.resolve(null);
          },
        },
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      });
    },
    { instance: INSTANCE, entry: ENTRY, item: REGISTRY_ITEM, healthReport: report(missingFiles) },
  );
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Test Mod' })).toBeVisible({ timeout: 15_000 });
}

test('a listed mod whose file is missing says so and offers to reinstall it', async ({ page }) => {
  await boot(page, ['test-mod.jar']);
  // Not "Installed ✓": the Health Check reports the file is not in mods/.
  await expect(page.getByRole('button', { name: 'Missing file · Reinstall' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Installed ✓ · Change version' })).toHaveCount(0);
  await expect(page.getByRole('alert')).toContainText('Missing file in Test Instance');
});

test('a listed mod whose file is present still reads as installed', async ({ page }) => {
  await boot(page, []);
  await expect(page.getByRole('button', { name: 'Installed ✓ · Change version' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Missing file · Reinstall' })).toHaveCount(0);
  await expect(page.getByRole('alert')).toHaveCount(0);
});
