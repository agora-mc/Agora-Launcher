import { test, expect, type Page } from '@playwright/test';

/**
 * Text size at 150% must not push pages sideways or stack controls on top of
 * each other. Checked at a small window, where scaled text has the least room.
 */
const INSTANCES = [
  { instance_id: 'a', name: 'Test Instance', minecraft_version: '1.21', loader: 'fabric', loader_version: '0.16.9', is_locked: false, last_launched_at: '2026-07-12T10:00:00Z' },
  { instance_id: 'b', name: 'A Very Long Instance Name For Overflow Checking Purposes Here', minecraft_version: '1.20.1', loader: 'neoforge', loader_version: '47.1.0', is_locked: true, last_launched_at: null },
];
const DETAIL = {
  row: { ...INSTANCES[0], is_modpack: false, jvm_memory_mb: 4096, jvm_gc: 'G1GC', jvm_custom_args: '', created_at: '2026-06-01T00:00:00Z' },
  manifest: {
    instance_id: 'a', name: 'Test Instance', created_from_pack: null, minecraft_version: '1.21', loader: 'fabric',
    loader_version: '0.16.9', is_locked: false, mods: [], resourcepacks: [], shaders: [], datapacks: [], worlds: [],
    user_preferences: { java_memory_gb: 4 },
  },
};

async function boot(page: Page, route: 'instance-detail' | null = null) {
  await page.setViewportSize({ width: 1024, height: 700 });
  await page.addInitScript(({ instances, detail, route }) => {
    localStorage.setItem('agora-ui-preferences', JSON.stringify({ version: 1, fontScale: 1.5 }));
    if (route) window.history.replaceState({ __agora: { type: route, instanceId: 'a' } }, '', '');
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: {
        transformCallback() { return 1; },
        unregisterCallback() {},
        invoke(command: string, args: Record<string, unknown> = {}) {
          if (command === 'get_setting') return Promise.resolve(args.key === 'onboarding_complete');
          if (command === 'get_registry_status') return Promise.resolve({ has_cached_db: true, cached_tag: 't', cached_schema_version: 5, latest_tag: 't', update_available: false, checked: true, message: 'Catalog ready.' });
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);
          if (command === 'list_instances') return Promise.resolve(instances);
          if (command === 'get_instance_detail') return Promise.resolve(detail);
          if (command === 'list_instance_content') return Promise.resolve([]);
          if (/^(list_|for_you|search_|query_launch_state)/.test(command)) return Promise.resolve([]);
          return Promise.resolve(null);
        },
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  }, { instances: INSTANCES, detail: DETAIL, route });
  await page.goto('/');
  await expect(page.locator('html')).toHaveCSS('--font-scale', '1.5');
}

async function expectNoSidewaysScroll(page: Page) {
  const { scrollWidth, clientWidth } = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
  }));
  expect(scrollWidth).toBeLessThanOrEqual(clientWidth);
}

/** No two visible buttons in the main column may share pixels. */
async function expectButtonsDoNotOverlap(page: Page) {
  const overlaps = await page.evaluate(() => {
    const boxes = [...document.querySelectorAll('main button, main a[href]')]
      .map((el) => ({ el, r: el.getBoundingClientRect() }))
      .filter(({ r }) => r.width > 0 && r.height > 0 && r.bottom > 0 && r.top < window.innerHeight);
    const hits: string[] = [];
    for (let i = 0; i < boxes.length; i += 1) {
      for (let j = i + 1; j < boxes.length; j += 1) {
        const a = boxes[i];
        const b = boxes[j];
        if (a.el.contains(b.el) || b.el.contains(a.el)) continue;
        const w = Math.min(a.r.right, b.r.right) - Math.max(a.r.left, b.r.left);
        const h = Math.min(a.r.bottom, b.r.bottom) - Math.max(a.r.top, b.r.top);
        if (w > 2 && h > 2) hits.push(`${a.el.textContent?.trim()} x ${b.el.textContent?.trim()}`);
      }
    }
    return hits;
  });
  expect(overlaps).toEqual([]);
}

for (const [name, nav] of [['Home', 'Home'], ['Browse', 'Browse'], ['My Instances', 'My Instances'], ['Settings', 'Settings']] as const) {
  test(`${name} fits at 150% text size`, async ({ page }) => {
    await boot(page);
    await page.getByRole('button', { name: nav, exact: true }).first().click();
    await page.waitForTimeout(300);
    await expectNoSidewaysScroll(page);
    await expectButtonsDoNotOverlap(page);
  });
}

test('instance editor header fits at 150% text size', async ({ page }) => {
  await boot(page, 'instance-detail');
  await expect(page.getByRole('heading', { name: 'Test Instance' })).toBeVisible();
  await expectNoSidewaysScroll(page);
  await expectButtonsDoNotOverlap(page);
});

test('instance cards give long names room at 150% text size', async ({ page }) => {
  await boot(page);
  await page.getByRole('button', { name: 'My Instances', exact: true }).first().click();
  const heading = page.getByRole('heading', { name: /A Very Long Instance Name/ });
  await expect(heading).toBeVisible();
  const box = await heading.boundingBox();
  // Word-per-line wrapping made this card several lines tall.
  expect(box!.height).toBeLessThan(120);
});

// ---------------------------------------------------------------------------
// Populated instance editor and the install review dialog
// ---------------------------------------------------------------------------

const MOD_NAMES = [
  'Sodium', 'An Extremely Long Mod Display Name That Keeps Going And Going',
  'Lithium', 'Iris Shaders Companion Extended Edition', 'Disabled Example Mod', 'Fabric API',
];
const SHA = 'a'.repeat(64);
const FILE = (i: number) => `some-very-long-mod-filename-for-overflow-checks-fabric-0.6.1${i}+mc1.21.1-build.${i}.jar`;
const CONTENT = MOD_NAMES.map((name, i) => ({
  key: `mod:${FILE(i)}:${SHA}`, filename: FILE(i), display_name: name, version: `0.6.1${i}`, content_type: 'mod',
  enabled: i !== 4, installed_at: '2026-07-01T00:00:00Z', source: 'modrinth', source_label: 'Modrinth', source_url: null,
  registry_id: null, modrinth_id: `p${i}`, mod_jar_id: `mod-${i}`, loader_mod_id: `mod-${i}`, size_bytes: 123456,
  file_present: true, resolved_path: null, author: 'Some Author', categories: ['Optimization'], icon_url: null,
  curation_status: 'unknown', agora_score: null, modrinth_downloads: 1000, metadata_status: 'unavailable',
}));
const UPDATES = [0, 1, 2].map((i) => ({
  filename: FILE(i), mod_jar_id: `mod-${i}`, current_version: `0.6.1${i}`, latest_version: '0.7.0', target_version: '0.7.0', source: 'modrinth',
}));

function richPlan() {
  const hash = { values: [{ algorithm: 'sha256', value: SHA }] };
  return {
    fingerprint: 'fp-rich',
    intent: {
      action: { type: 'batch-update', items: [] }, targetInstance: 'a', optionalDeps: { type: 'prompt' },
      requestedBy: 'interactive', overrides: { allowReplace: false, skipHealthScan: false, forceConflictResolution: {} },
    },
    operation: { type: 'batch-update', operations: [] },
    dependencies: [
      { modJarId: 'required-dep-with-a-very-long-identifier', displayName: 'Required Dependency With A Very Long Display Name', requirement: 'required', source: 'jar', disposition: { type: 'install-candidate', artifact: {} } },
      { modJarId: 'optional-dep-b', requirement: 'optional', source: 'jar', disposition: { type: 'install-candidate', artifact: {} } },
    ],
    conflicts: [{
      conflictId: 'c1', kind: 'duplicate-mod', existingModJarId: 'existing-mod', incomingModJarId: 'incoming-mod',
      message: 'incoming-mod conflicts with existing-mod, they provide the same features and cannot both be loaded.',
      blocking: true, resolutionOptions: ['replace', 'skip'],
    }],
    filesToAdd: [{ targetFilename: FILE(1), stagingFilename: 's.jar', artifact: {}, hashes: hash, size: 250000 }],
    filesToRemove: [], filesToDisable: [],
    snapshot: { label: 'Before updating', estimatedBytes: 500000 },
    diskEstimate: { downloadBytes: 250000, snapshotBytes: 500000, applyOverheadBytes: 100000, peakAdditionalBytes: 600000, postCommitDeltaBytes: 250000 },
    warnings: [{ code: 'w1', message: 'This version was built for a different Minecraft patch release and may behave unexpectedly in your instance.' }],
    blockingErrors: [{ code: 'e1', message: 'Required dependency required-dep-with-a-very-long-identifier has no version compatible with Minecraft 1.21.' }],
    pendingChoices: [
      { type: 'optional-dependencies', choiceId: 'opt', options: [{ modJarId: 'optional-dep-b', displayName: 'Optional Dep B' }] },
      { type: 'conflict', choiceId: 'cc1', conflictId: 'c1', options: ['replace', 'skip'] },
    ],
    createdAt: '2026-07-12T17:00:00Z', instanceStateHash: 'abc', registryRevision: 'v1',
  };
}

async function bootEditor(page: Page, width: number, height: number) {
  await page.setViewportSize({ width, height });
  await page.addInitScript(({ instances, detail, content, updates, plan }) => {
    localStorage.setItem('agora-ui-preferences', JSON.stringify({ version: 1, fontScale: 1.5 }));
    window.history.replaceState({ __agora: { type: 'instance-detail', instanceId: 'a' } }, '', '');
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: {
        transformCallback() { return 1; },
        unregisterCallback() {},
        invoke(command: string, args: Record<string, unknown> = {}) {
          if (command === 'get_setting') return Promise.resolve(args.key === 'onboarding_complete');
          if (command === 'get_registry_status') return Promise.resolve({ has_cached_db: true, cached_tag: 't', cached_schema_version: 5, latest_tag: 't', update_available: false, checked: true, message: 'Catalog ready.' });
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);
          if (command === 'list_instances') return Promise.resolve(instances);
          if (command === 'get_instance_detail') return Promise.resolve(detail);
          if (command === 'list_instance_content') return Promise.resolve(content);
          if (command === 'check_instance_updates') return Promise.resolve(updates);
          if (command === 'resolve_install_plan') return Promise.resolve(plan);
          if (/^(list_|for_you|search_|query_launch_state)/.test(command)) return Promise.resolve([]);
          return Promise.resolve(null);
        },
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  }, { instances: INSTANCES, detail: DETAIL, content: CONTENT, updates: UPDATES, plan: richPlan() });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: /Installed Mods/ })).toBeVisible({ timeout: 15_000 });
}

/** Optional: TEXT_SCALE_SHOTS=<dir> writes a screenshot per run for eyeballing. */
async function maybeShoot(page: Page, name: string) {
  const dir = process.env.TEXT_SCALE_SHOTS;
  if (dir) await page.screenshot({ path: `${dir}/${name}-${page.viewportSize()!.width}.png` });
}

for (const [w, h] of [[1024, 700], [1280, 800]] as const) {
  test(`populated instance editor fits at 150% text size (${w}x${h})`, async ({ page }) => {
    await bootEditor(page, w, h);
    await page.getByRole('button', { name: 'Check for updates' }).click();
    await expect(page.getByRole('button', { name: /^Update all/ })).toBeVisible();
    await maybeShoot(page, 'editor-mods');
    await page.getByRole('table').first().scrollIntoViewIfNeeded();
    await maybeShoot(page, 'editor-table');
    await expectNoSidewaysScroll(page);
    await expectButtonsDoNotOverlap(page);
  });

  test(`install review dialog fits at 150% text size (${w}x${h})`, async ({ page }) => {
    await bootEditor(page, w, h);
    await page.getByRole('button', { name: 'Check for updates' }).click();
    await page.getByRole('button', { name: /^Update all/ }).click();
    const panel = page.getByRole('dialog');
    await expect(panel.getByText('Review Instance Changes')).toBeVisible();
    await maybeShoot(page, 'install-review');
    const box = await panel.boundingBox();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(w);
    // Nothing inside the dialog may spill past its right edge.
    const spill = await panel.evaluate((el) => {
      const right = el.getBoundingClientRect().right + 1;
      return [...el.querySelectorAll('*')].filter((n) => n.getBoundingClientRect().right > right && n.getBoundingClientRect().width > 0).length;
    });
    expect(spill).toBe(0);
    const overlaps = await panel.evaluate((el) => {
      const bs = [...el.querySelectorAll('button')].map((b) => ({ b, r: b.getBoundingClientRect() })).filter(({ r }) => r.width > 0);
      let n = 0;
      for (let i = 0; i < bs.length; i += 1) {
        for (let j = i + 1; j < bs.length; j += 1) {
          const a = bs[i];
          const c = bs[j];
          if (a.b.contains(c.b) || c.b.contains(a.b)) continue;
          if (Math.min(a.r.right, c.r.right) - Math.max(a.r.left, c.r.left) > 2 && Math.min(a.r.bottom, c.r.bottom) - Math.max(a.r.top, c.r.top) > 2) n += 1;
        }
      }
      return n;
    });
    expect(overlaps).toBe(0);
  });
}
