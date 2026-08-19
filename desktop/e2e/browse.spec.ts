import { test, expect, type Page } from '@playwright/test';

type BrowseResult = { items: unknown[]; total: number; page: number; hasMore: boolean };

const item = (id: string, name: string) => ({
  id,
  source: 'curated',
  registryItem: {
    id,
    name,
    content_type: 'mod',
    download_strategy: 'github_release',
    upvotes: 0,
    downvotes: 0,
    net_score: 0,
  },
  modrinthResult: null,
  name,
  iconUrl: null,
  description: null,
  contentType: 'mod',
  heroImageUrl: null,
  author: null,
  categories: [],
  downloads: null,
  follows: null,
  upvotes: 0,
  downvotes: 0,
  netScore: 0,
  supportedVersions: [],
  sourcePageUrl: null,
});

async function installBrowseMock(page: Page) {
  await page.addInitScript(() => {
    const calls: Array<{
      command: string;
      args: Record<string, unknown>;
      resolve: (value: unknown) => void;
      reject: (reason?: unknown) => void;
    }> = [];
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
          const key = args.key;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'modrinth_enabled') return Promise.resolve(true);
          return Promise.resolve(false);
        }
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
        if (command === 'list_categories') {
          return Promise.resolve([
            { id: 'performance', display_name: 'Performance', is_community: false, content_types: ['mod'] },
            { id: 'questing', display_name: 'Questing', is_community: false, content_types: ['pack'] },
            { id: 'visuals', display_name: 'Visuals', is_community: true, content_types: ['mod', 'shader'] },
          ]);
        }
        if (command === 'list_modrinth_categories') {
          return Promise.resolve([
            { name: 'technology', project_type: 'mod', header: 'categories' },
            { name: 'adventure', project_type: 'mod', header: 'categories' },
            { name: 'adventure', project_type: 'modpack', header: 'categories' },
            { name: 'kitchen-sink', project_type: 'modpack', header: 'categories' },
            { name: 'realistic', project_type: 'shader', header: 'features' },
            { name: 'audio', project_type: 'resourcepack', header: 'features' },
            { name: 'worldgen', project_type: 'datapack', header: 'categories' },
            { name: 'minigame', project_type: 'minecraft_java_server', header: 'minecraft_server_gameplay' },
          ]);
        }
        if (command === 'list_manifest_loaders' || command === 'list_manifest_mc_versions') {
          return Promise.resolve([]);
        }
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'list_instances') return Promise.resolve([]);
        if (command === 'list_snapshots') return Promise.resolve([]);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command === 'browse_search' || command === 'browse_load_more') {
          return new Promise((resolve, reject) => calls.push({ command, args, resolve, reject }));
        }
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __browseCalls: calls,
      __resolveBrowse(index: number, value: unknown) { calls[index].resolve(value); },
      __rejectBrowse(index: number, value: unknown) { calls[index].reject(value); },
    });
  });
}

async function waitForCalls(page: Page, count: number) {
  await expect.poll(() => page.evaluate(() => (window as any).__browseCalls.length)).toBeGreaterThanOrEqual(count);
}

async function findCall(page: Page, command: string, query?: string, excluded: number[] = []) {
  let index = -1;
  await expect.poll(async () => {
    index = await page.evaluate(({ command, query, excluded }) => {
      const calls = (window as any).__browseCalls as Array<{ command: string; args: Record<string, unknown> }>;
      return calls.findIndex((call, i) =>
        !excluded.includes(i)
        && call.command === command
        && (query === undefined || call.args.query === query),
      );
    }, { command, query, excluded });
    return index;
  }).toBeGreaterThanOrEqual(0);
  return index;
}

async function callArg(page: Page, index: number, key: string) {
  return page.evaluate(
    ([i, k]) => (window as any).__browseCalls[i as number].args[k as string],
    [index, key] as const,
  );
}

async function resolveCall(page: Page, index: number, result: BrowseResult) {
  await page.evaluate(({ index, result }) => (window as any).__resolveBrowse(index, result), { index, result });
}

async function openBrowse(page: Page) {
  await page.goto('/');
  await page.getByRole('button', { name: 'Browse', exact: true }).click();
  // React StrictMode intentionally runs mount effects twice in development.
  await waitForCalls(page, 2);
  return page.evaluate(() => (window as any).__browseCalls.length - 1) as Promise<number>;
}

test('out-of-order searches only display the newest query', async ({ page }) => {
  await installBrowseMock(page);
  const initial = await openBrowse(page);
  await resolveCall(page, initial, { items: [item('initial', 'Initial')], total: 1, page: 0, hasMore: false });

  const search = page.getByPlaceholder('Search mods, packs, and more…');
  await search.fill('alpha');
  const alpha = await findCall(page, 'browse_search', 'alpha');
  await search.fill('beta');
  const beta = await findCall(page, 'browse_search', 'beta');

  await resolveCall(page, beta, { items: [item('beta', 'Beta Result')], total: 1, page: 0, hasMore: false });
  await expect(page.getByText('Beta Result')).toBeVisible();
  await resolveCall(page, alpha, { items: [item('alpha', 'Alpha Result')], total: 1, page: 0, hasMore: false });

  await expect(page.getByText('Beta Result')).toBeVisible();
  await expect(page.getByText('Alpha Result')).toHaveCount(0);
});

test('stale pagination is ignored and new query can paginate', async ({ page }) => {
  await installBrowseMock(page);
  const initial = await openBrowse(page);
  await resolveCall(page, initial, { items: [item('a', 'Query A')], total: 40, page: 0, hasMore: true });
  await page.getByTestId('browse-load-sentinel').scrollIntoViewIfNeeded();
  const staleLoad = await findCall(page, 'browse_load_more');

  const search = page.getByPlaceholder('Search mods, packs, and more…');
  await search.fill('beta');
  const beta = await findCall(page, 'browse_search', 'beta');
  await resolveCall(page, beta, { items: [item('b', 'Query B')], total: 40, page: 0, hasMore: true });
  await resolveCall(page, staleLoad, { items: [item('a-more', 'Stale A Page')], total: 40, page: 1, hasMore: false });

  await page.getByTestId('browse-load-sentinel').scrollIntoViewIfNeeded();
  const betaLoad = await findCall(page, 'browse_load_more', undefined, [staleLoad]);
  const args = await page.evaluate((index) => (window as any).__browseCalls[index].args, betaLoad);
  expect(args.queryKey).toContain('beta');
  await resolveCall(page, betaLoad, { items: [item('b', 'Query B'), item('b-more', 'Query B Page')], total: 40, page: 1, hasMore: false });

  await expect(page.getByText('Stale A Page')).toHaveCount(0);
  await expect(page.getByText('Query B Page')).toBeVisible();
  await expect(page.getByText('Query B', { exact: true })).toHaveCount(1);
});

test('pagination failure is visible and retryable', async ({ page }) => {
  await installBrowseMock(page);
  const initial = await openBrowse(page);
  await resolveCall(page, initial, { items: [item('a', 'Initial Page')], total: 40, page: 0, hasMore: true });
  await page.getByTestId('browse-load-sentinel').scrollIntoViewIfNeeded();
  const failedLoad = await findCall(page, 'browse_load_more');
  await page.evaluate((index) => (window as any).__rejectBrowse(index, new Error('Pagination failed')), failedLoad);

  await expect(page.getByText('Pagination failed')).toBeVisible();
  await page.getByRole('button', { name: 'Retry loading more' }).click();
  const retry = await findCall(page, 'browse_load_more', undefined, [failedLoad]);
  await resolveCall(page, retry, { items: [item('more', 'Next Page')], total: 40, page: 1, hasMore: false });
  await expect(page.getByText('Next Page')).toBeVisible();
  await expect(page.getByText('Pagination failed')).toHaveCount(0);
});

test('tile media is bounded and unmounted in list mode', async ({ page }) => {
  await installBrowseMock(page);
  const initial = await openBrowse(page);
  const galleryItem = {
    ...item('gallery', 'Gallery Project'),
    heroImageUrl: 'https://cdn.modrinth.com/data/example/images/hero.png',
    iconUrl: 'https://cdn.modrinth.com/data/example/icon.png',
    description: 'A project with gallery artwork.',
  };
  await resolveCall(page, initial, { items: [galleryItem], total: 1, page: 0, hasMore: false });
  const searchCallsBeforeLayoutChange = await page.evaluate(() => (window as any).__browseCalls.length);

  await page.getByTitle('Grid view').click();
  const tile = page.locator('.browse-tile-card');
  await expect(tile.locator('.browse-hero-media__image')).toHaveAttribute('loading', 'lazy');
  expect(await tile.evaluate((element) => element.getBoundingClientRect().width)).toBeLessThanOrEqual(330.5);
  expect(await tile.locator('.browse-hero-media').evaluate((element) => element.getBoundingClientRect().height)).toBeLessThanOrEqual(180.5);

  await page.getByTitle('List view').click();
  await expect(page.locator('.browse-hero-media__image')).toHaveCount(0);
  await expect(page.getByTestId('browse-list-results')).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__browseCalls.length)).toBe(searchCallsBeforeLayoutChange);
});

test('list columns respond to available width and keep icons bounded', async ({ page }) => {
  await page.setViewportSize({ width: 2200, height: 1200 });
  await installBrowseMock(page);
  const initial = await openBrowse(page);
  const items = Array.from({ length: 10 }, (_, index) => ({
    ...item(`responsive-${index}`, `Responsive ${index}`),
    description: 'A detailed description that uses the additional width available in list mode.',
    categories: index === 0 ? ['technology', 'fabric'] : [],
    supportedVersions: index === 0
      ? ['1.14', '1.16.5', '1.18.2', '1.19.4', '1.20.1', '1.21', '1.21.8']
      : [],
  }));
  await resolveCall(page, initial, { items, total: items.length, page: 0, hasMore: false });

  const wideColumns = await page.locator('.browse-list-card').evaluateAll((cards) =>
    new Set(cards.map((card) => Math.round(card.getBoundingClientRect().left))).size,
  );
  expect(wideColumns).toBe(2);
  expect(await page.locator('.browse-list-card').first().evaluate((card) => card.getBoundingClientRect().width)).toBeGreaterThan(700);
  const iconWidth = await page.locator('.browse-list-card__icon').first().evaluate((icon) => icon.getBoundingClientRect().width);
  expect(iconWidth).toBeGreaterThanOrEqual(48);
  expect(iconWidth).toBeLessThanOrEqual(72);
  await expect(page.getByText('technology', { exact: true })).toBeVisible();
  await expect(page.getByText('MC 1.14–1.21.8 · 7 supported versions')).toBeVisible();

  await page.setViewportSize({ width: 650, height: 900 });
  const narrowColumns = await page.locator('.browse-list-card').evaluateAll((cards) =>
    new Set(cards.map((card) => Math.round(card.getBoundingClientRect().left))).size,
  );
  expect(narrowColumns).toBe(1);
});

test('broken tile artwork falls back to a project initial', async ({ page }) => {
  await page.route('https://cdn.modrinth.com/**', (route) => route.abort());
  await installBrowseMock(page);
  const initial = await openBrowse(page);
  await resolveCall(page, initial, {
    items: [{
      ...item('broken-gallery', 'Gallery Failure'),
      heroImageUrl: 'https://cdn.modrinth.com/data/example/images/missing.png',
      iconUrl: 'https://cdn.modrinth.com/data/example/missing-icon.png',
    }],
    total: 1,
    page: 0,
    hasMore: false,
  });

  await page.getByTitle('Grid view').click();
  await expect(page.locator('.browse-hero-media .browse-icon--placeholder')).toHaveText('G');
});

test('category lists follow the selected content type', async ({ page }) => {
  await installBrowseMock(page);
  const initial = await openBrowse(page);
  await resolveCall(page, initial, { items: [], total: 0, page: 0, hasMore: false });

  // Default content type is 'mod' — only mod categories are shown initially.
  await expect(page.getByRole('button', { name: 'Technology', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Kitchen Sink', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Adventure', exact: true })).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Minigame', exact: true })).toHaveCount(0);

  await page.getByLabel('Content type').selectOption('pack');

  await expect(page.getByText('Categories for pack content.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Kitchen Sink', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Adventure', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Technology', exact: true })).toHaveCount(0);

  await page.getByLabel('Content type').selectOption('server');
  await expect(page.getByRole('button', { name: 'Minigame', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Kitchen Sink', exact: true })).toHaveCount(0);
});

test('curated category dropdown is searchable and type-aware', async ({ page }) => {
  await installBrowseMock(page);
  const initial = await openBrowse(page);
  await resolveCall(page, initial, { items: [], total: 0, page: 0, hasMore: false });
  await page.getByLabel('Content type').selectOption('pack');

  await page.getByRole('button', { name: 'Curated categories' }).click();
  const categorySearch = page.getByLabel('Search curated categories');
  await categorySearch.fill('perf');
  await expect(page.getByText('No curated categories found.')).toBeVisible();
  await categorySearch.fill('quest');
  await expect(page.getByRole('menuitem', { name: 'Questing' })).toBeVisible();
  await page.getByRole('menuitem', { name: 'Questing' }).click();

  await expect(page.getByRole('button', { name: 'Curated category: Questing' })).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const calls = (window as any).__browseCalls as Array<{ command: string; args: Record<string, unknown> }>;
    return calls.some((call) => call.command === 'browse_search'
      && call.args.contentType === 'pack'
      && call.args.category === 'questing');
  })).toBe(true);
});

// ---------------------------------------------------------------------------
// D1: Browse instance-context selector and compatibility labels
// ---------------------------------------------------------------------------

const CONTEXT_INSTANCE: Record<string, unknown> = {
  instance_id: 'fabric-121',
  name: 'My Fabric World',
  minecraft_version: '1.21',
  loader: 'fabric',
  loader_version: '0.16.9',
  is_modpack: false,
  is_locked: false,
  last_launched_at: '2026-07-12T08:00:00Z',
  jvm_memory_mb: 4096,
  jvm_gc: 'G1GC',
  jvm_custom_args: '',
  created_at: '2026-06-01T00:00:00Z',
};

const CONTEXT_DETAIL: Record<string, unknown> = {
  row: CONTEXT_INSTANCE,
  manifest: {
    instance_id: 'fabric-121',
    name: 'My Fabric World',
    created_from_pack: null,
    minecraft_version: '1.21',
    loader: 'fabric',
    loader_version: '0.16.9',
    is_locked: false,
    mods: [
      { filename: 'already-installed.jar', registry_id: 'installed-mod', modrinth_id: null, source: 'curated', version: '1.0.0', sha256: 'a', installed_at: '2026-07-01T00:00:00Z', mod_jar_id: 'installed-mod', enabled: true, content_type: 'mod' },
    ],
    resourcepacks: [],
    shaders: [],
    datapacks: [],
    worlds: [],
    user_preferences: {},
  },
};

async function installBrowseContextMock(page: Page) {
  // Serialize fixture data as params so addInitScript's serialized closure
  // can access them (module-level variables are not captured).
  const instanceData = CONTEXT_INSTANCE;
  const detailData = CONTEXT_DETAIL;

  await page.addInitScript(
    (params: { instance: Record<string, unknown>; detail: Record<string, unknown> }) => {
      const { instance, detail } = params;
      const callbacks = new Map<number, (...args: unknown[]) => void>();
      let callbackId = 0;
      let updateChecks = 0;

      // Helper inlined because addInitScript only serializes the function body.
      function compatItem(id: string, name: string): Record<string, unknown> {
        return {
          id, source: 'curated',
          registryItem: {
            id, name, content_type: 'mod', download_strategy: 'github_release',
            source_identifier: `${id}/releases`, sha256: '', upvotes: 5, downvotes: 0,
            net_score: 5, velocity: 0, status: 'active', is_immune: false,
            immunity_reason: null, allow_comments: true, icon_url: null,
            gallery_urls_json: null, date_added: '2026-06-01',
            compatible_versions_json: JSON.stringify([{ mc_version: '1.21', loader: 'fabric', mod_version: '1.0.0' }]),
            description: null, body_markdown: null, page_url: null, license_id: 'MIT',
            source_updated_at: '2026-07-01T00:00:00Z', modrinth_id: null,
            recommendation_reason: null, recommendation_overlap: null,
          },
          modrinthResult: null, name, iconUrl: null, description: null, contentType: 'mod',
          heroImageUrl: null, author: null, categories: [], downloads: null, follows: null,
          upvotes: 5, downvotes: 0, netScore: 5, supportedVersions: ['1.21'], sourcePageUrl: null,
        };
      }

      const internals = {
        transformCallback(callback: (...args: unknown[]) => void) {
          const id = ++callbackId;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback(id: number) { callbacks.delete(id); },
        invoke(command: string, args: Record<string, unknown> = {}) {
          // Settings
          if (command === 'get_setting') {
            const key = args.key as string;
            if (key === 'onboarding_complete') return Promise.resolve(true);
            if (key === 'modrinth_enabled') return Promise.resolve(true);
            if (key === 'ai_chat_enabled') return Promise.resolve(false);
            if (key === 'ai_mcp_enabled') return Promise.resolve(false);
            return Promise.resolve(null);
          }

          // Registry
          if (command === 'get_registry_status') {
            return Promise.resolve({
              has_cached_db: true, cached_tag: 'test', cached_schema_version: 5,
              latest_tag: 'test', update_available: false, checked: true,
              message: 'Registry ready.',
            });
          }
          if (command === 'check_registry_update') {
            return Promise.resolve({
              has_cached_db: true, cached_tag: 'test', cached_schema_version: 5,
              latest_tag: 'test', update_available: false, checked: true,
              message: 'Registry ready.',
            });
          }
          if (command === 'list_categories') return Promise.resolve([]);
          if (command === 'list_manifest_loaders') return Promise.resolve(['fabric', 'forge', 'quilt']);
          if (command === 'list_manifest_mc_versions') return Promise.resolve(['1.20.1', '1.21', '1.21.1']);

          // Browse search — immediate (not deferred) for context tests
          if (command === 'browse_search') {
            return Promise.resolve({
              items: [
                compatItem('exact-mod', 'Exact Mod'),
                compatItem('major-mod', 'Major Match Mod'),
                compatItem('installed-mod', 'Already Installed'),
                compatItem('updatable-mod', 'Updatable Mod'),
              ],
              total: 4,
              page: 0,
              hasMore: false,
            });
          }
          if (command === 'browse_load_more') {
            return Promise.resolve({ items: [], total: 0, page: 1, hasMore: false });
          }

          // For You items
          if (command === 'for_you_items') {
            return Promise.resolve([
              {
                id: 'rec-for-you',
                name: 'For You Mod',
                content_type: 'mod',
                download_strategy: 'github_release',
                source_identifier: 'rec-for-you/releases',
                sha256: '',
                upvotes: 10,
                downvotes: 0,
                net_score: 10,
                velocity: 1.5,
                status: 'active',
                is_immune: false,
                immunity_reason: null,
                allow_comments: true,
                icon_url: null,
                gallery_urls_json: null,
                date_added: '2026-07-01',
                compatible_versions_json: null,
                description: 'A personalized recommendation.',
                body_markdown: null,
                page_url: null,
                license_id: 'MIT',
                source_updated_at: '2026-07-10T00:00:00Z',
                modrinth_id: null,
                recommendation_reason: 'Recommended by Agora\'s curated score for fabric 1.21.',
                recommendation_overlap: 5,
              },
            ]);
          }

          // Instances — use the params passed from the outer scope
          if (command === 'list_instances') {
            return Promise.resolve([instance]);
          }
          if (command === 'get_instance_detail') {
            return Promise.resolve(detail);
          }
          if (command === 'check_instance_updates') {
            updateChecks += 1;
            return Promise.resolve([
              {
                filename: 'updatable-mod.jar',
                mod_jar_id: 'updatable-mod',
                current_version: '1.0.0',
                latest_version: '2.0.0',
                target_version: '2.0.0',
                source: 'curated',
              },
            ]);
          }

          // Misc
          if (command === 'get_windows_accent_color') return Promise.resolve(null);
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);
          if (command === 'get_auth_status') return Promise.resolve(true);
          if (command === 'get_github_profile') return Promise.resolve(null);
          if (command === 'get_flag_rate_limit') return Promise.resolve(null);

          return Promise.resolve(null);
        },
      };
      Object.assign(window as unknown as Record<string, unknown>, {
        __TAURI_INTERNALS__: internals,
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
        __browseUpdateChecks: () => updateChecks,
      });
    },
    { instance: instanceData, detail: detailData },
  );
}

test.describe('D1 — Browse instance-context selector', () => {

  test('instance selector shows installed status without compatibility labels', async ({ page }) => {
    await installBrowseContextMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Browse', exact: true }).click();

    // Wait for the items to render
    await expect(page.getByText('Exact Mod')).toBeVisible({ timeout: 5000 });

    // Select the instance from the "Discover for an instance" dropdown
    const contextSelect = page.locator('#browse-instance-context');
    await contextSelect.selectOption('fabric-121');

    await expect(page.getByText('Installed').first()).toBeVisible({ timeout: 5000 });
    await expect(page.getByText(/Compatible with|May work with/)).toHaveCount(0);
  });

  test('uninstalled items have no instance-status label', async ({ page }) => {
    await installBrowseContextMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Browse', exact: true }).click();

    await expect(page.getByText('Exact Mod')).toBeVisible();

    const contextSelect = page.locator('#browse-instance-context');
    await contextSelect.selectOption('fabric-121');

    await expect(page.getByText(/Compatible with|May work with/)).toHaveCount(0);
  });

  test('installed label shown for items in the active instance manifest', async ({ page }) => {
    await installBrowseContextMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Browse', exact: true }).click();

    await expect(page.getByText('Already Installed')).toBeVisible();

    const contextSelect = page.locator('#browse-instance-context');
    await contextSelect.selectOption('fabric-121');

    // "Already Installed" has registry_id = 'installed-mod' which matches item.id
    await expect(page.getByText('Installed').first()).toBeVisible({ timeout: 5000 });
  });

  test('selecting an instance does not check mod updates', async ({ page }) => {
    await installBrowseContextMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Browse', exact: true }).click();

    await expect(page.getByText('Updatable Mod')).toBeVisible();

    const contextSelect = page.locator('#browse-instance-context');
    await contextSelect.selectOption('fabric-121');

    await expect(page.getByText(/Compatible with|May work with/)).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__browseUpdateChecks())).toBe(0);
    await expect(page.getByText('Update available')).toHaveCount(0);
  });

  test('For You sort shows per-item recommendation reason', async ({ page }) => {
    await installBrowseContextMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Browse', exact: true }).click();

    // Select the instance first — activeInstance is needed for contextFor to
    // return the recommendation reason.
    const contextSelect = page.locator('#browse-instance-context');
    await contextSelect.selectOption('fabric-121');
    await expect(page.getByText('Installed').first()).toBeVisible({ timeout: 5000 });

    // Now switch sort to "For You"
    const selects = page.locator('select');
    const count = await selects.count();
    let sortFound = false;
    for (let i = 0; i < count; i++) {
      const options = await selects.nth(i).locator('option').allTextContents();
      if (options.includes('For You')) {
        await selects.nth(i).selectOption('for_you');
        sortFound = true;
        break;
      }
    }
    if (!sortFound) {
      // Fallback: last select is the sort selector
      await selects.last().selectOption('for_you');
    }

    // Wait for the For You item to appear (from forYouItems mock)
    await expect(page.getByText('For You Mod')).toBeVisible({ timeout: 5000 });

    // The item's registryItem has recommendation_reason set, and activeInstance
    // is populated, so the "Why:" text appears.
    await expect(
      page.getByText(/Why: Recommended by Agora's curated score for fabric 1\.21/),
    ).toBeVisible();
  });

});

// ---------------------------------------------------------------------------
// E3: Bulk select and install from browse cards
// ---------------------------------------------------------------------------

const BULK_ITEM_A = item('bulk-mod-a', 'Bulk Mod A');
const BULK_ITEM_B = item('bulk-mod-b', 'Bulk Mod B');
const BULK_ITEM_M = { ...item('mr-project-1', 'Bulk Modrinth'), source: 'modrinth' as const };
// Matches the installed entry in CONTEXT_DETAIL.manifest.mods.
const BULK_ITEM_I = item('installed-mod', 'Installed Mod');

const bulkCard = (page: Page, name: string) =>
  page.locator('.browse-list-card').filter({ hasText: name });

function bulkBatchPlan(intent: unknown) {
  return {
    fingerprint: 'plan-fp-bulk-001',
    intent,
    operation: { type: 'batch-install', operations: [] },
    dependencies: [],
    conflicts: [],
    filesToAdd: [],
    filesToRemove: [],
    filesToDisable: [],
    snapshot: { label: 'Before installing selected items', estimatedBytes: 1_000_000 },
    diskEstimate: { downloadBytes: 500_000, snapshotBytes: 1_000_000, applyOverheadBytes: 200_000, peakAdditionalBytes: 1_200_000, postCommitDeltaBytes: 500_000 },
    warnings: [],
    blockingErrors: [],
    pendingChoices: [],
    createdAt: '2026-08-01T00:00:00Z',
    instanceStateHash: 'bulkstatehash',
    registryRevision: 'v20260801',
  };
}

function bulkBatchPlanWithOptionalDep(intent: unknown) {
  return {
    ...bulkBatchPlan(intent),
    fingerprint: 'plan-fp-bulk-optional',
    dependencies: [
      {
        modJarId: 'required-dep',
        requirement: 'required',
        source: 'manifest',
        disposition: { type: 'install-candidate', artifact: {} },
        displayName: 'Required Dependency',
        pageUrl: 'https://modrinth.com/mod/required-dep',
      },
      {
        modJarId: 'optional-dep-b',
        requirement: 'optional',
        source: 'manifest',
        disposition: { type: 'excluded' },
        displayName: 'TerraBlender',
        pageUrl: 'https://modrinth.com/mod/terrablender',
      },
      {
        modJarId: 'batch-sibling-dep',
        requirement: 'required',
        source: 'manifest',
        disposition: { type: 'included-in-batch', targetFilename: 'bulk-mod-b.jar' },
        displayName: 'Bulk Mod B',
      },
    ],
    pendingChoices: [],
  };
}

function bulkBatchPlanBlocking(intent: unknown) {
  return {
    ...bulkBatchPlan(intent),
    fingerprint: 'plan-fp-bulk-blocking',
    blockingErrors: [
      { code: 'ERR_REQUIRED_DEPENDENCY', message: 'Required dependency missing-dep could not be resolved: no compatible artifact' },
    ],
  };
}

function bulkBatchPlanWithNewOptionalFile(intent: unknown) {
  return {
    ...bulkBatchPlan(intent),
    fingerprint: 'plan-fp-bulk-optional-final',
    filesToAdd: [{ targetFilename: 'optional-dep-b.jar' }],
  };
}

function bulkBatchPlanWithRootArtifact(intent: unknown) {
  return {
    ...bulkBatchPlan(intent),
    operation: {
      type: 'batch-install',
      operations: [{
        type: 'install',
        artifact: {
          type: 'download',
          itemId: 'bulk-mod-a',
          versionId: 'bulk-a-v1',
          filename: 'bulk-mod-a-1.0.0.jar',
          metadata: { sourceType: 'modrinth', version: '1.0.0' },
        },
      }],
    },
    filesToAdd: [{ targetFilename: 'bulk-mod-a-1.0.0.jar' }],
  };
}

function bulkBatchPlanHashBlocking(intent: unknown) {
  return {
    ...bulkBatchPlanWithRootArtifact(intent),
    blockingErrors: [{
      code: 'ERR_HASH_UNAVAILABLE',
      message: 'bulk-mod-a has no acceptable published hash for Curated source.',
    }],
  };
}

const BULK_SUCCESS_OUTCOME = {
  type: 'success',
  installedItems: ['bulk-mod-a-1.0.0.jar', 'bulk-mod-b-1.0.0.jar'],
  existingItemsReused: [],
  warnings: [],
  health: { type: 'completed', report: {} },
  snapshotId: 'snap-bulk-001',
};

async function installBulkSelectMock(page: Page, autoConfirmClean = true, alwaysAutoConfirm = false) {
  const instance = CONTEXT_INSTANCE;
  const detail = CONTEXT_DETAIL;
  const itemA = BULK_ITEM_A;
  const itemB = BULK_ITEM_B;
  const itemM = BULK_ITEM_M;
  const itemI = BULK_ITEM_I;

  await page.addInitScript(
    (params: {
      instance: Record<string, unknown>;
      detail: Record<string, unknown>;
      itemA: Record<string, unknown>;
      itemB: Record<string, unknown>;
      itemM: Record<string, unknown>;
      itemI: Record<string, unknown>;
      autoConfirmClean: boolean;
      alwaysAutoConfirm: boolean;
    }) => {
      const { instance, detail, itemA, itemB, itemM, itemI, autoConfirmClean, alwaysAutoConfirm } = params;
      const calls: Array<{
        command: string;
        args: Record<string, unknown>;
        resolve: (value: unknown) => void;
        reject: (reason?: unknown) => void;
      }> = [];
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
            const key = args.key;
            if (key === 'onboarding_complete') return Promise.resolve(true);
            if (key === 'modrinth_enabled') return Promise.resolve(true);
            if (key === 'ai_chat_enabled') return Promise.resolve(false);
            if (key === 'install_auto_confirm_clean') return Promise.resolve(autoConfirmClean);
            if (key === 'install_always_auto_confirm') return Promise.resolve(alwaysAutoConfirm);
            return Promise.resolve(null);
          }
          if (command === 'get_registry_status') {
            return Promise.resolve({
              has_cached_db: true, cached_tag: 'test', cached_schema_version: 5,
              latest_tag: 'test', update_available: false, checked: true,
              message: 'Registry ready.',
            });
          }
          if (command === 'list_categories') return Promise.resolve([]);
          if (command === 'list_manifest_loaders') return Promise.resolve([]);
          if (command === 'list_manifest_mc_versions') return Promise.resolve([]);
          if (command === 'get_windows_accent_color') return Promise.resolve(null);
          if (command === 'list_instances') return Promise.resolve([instance]);
          if (command === 'get_instance_detail') return Promise.resolve(detail);
          if (command === 'get_registry_item') {
            const itemId = args.itemId;
            return Promise.resolve(itemId === itemA.id ? itemA.registryItem : null);
          }
          if (command === 'browse_search') {
            return Promise.resolve({ items: [itemA, itemB, itemM, itemI], total: 4, page: 0, hasMore: false });
          }
          if (command === 'browse_load_more') {
            return Promise.resolve({ items: [], total: 4, page: 1, hasMore: false });
          }
          if (command === 'for_you_items') {
            return Promise.resolve([]);
          }
          if (command === 'resolve_install_plan' || command === 'apply_install_plan') {
            return new Promise((resolve, reject) => calls.push({ command, args, resolve, reject }));
          }
          if (command === 'cancel_install') return Promise.resolve(null);
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);
          if (command === 'get_auth_status') return Promise.resolve(false);
          return Promise.resolve(null);
        },
      };
      Object.assign(window as unknown as Record<string, unknown>, {
        __TAURI_INTERNALS__: internals,
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
        __bulkInstallCalls: calls,
        __resolveBulkInstall(index: number, value: unknown) { calls[index].resolve(value); },
        __rejectBulkInstall(index: number, value: unknown) { calls[index].reject(value); },
      });
    },
    { instance, detail, itemA, itemB, itemM, itemI, autoConfirmClean, alwaysAutoConfirm },
  );
}

async function findBulkInstallCall(page: Page, command: string) {
  let index = -1;
  await expect.poll(async () => {
    index = await page.evaluate((cmd) => {
      const calls = (window as any).__bulkInstallCalls as Array<{ command: string }>;
      return calls.findIndex((call) => call.command === cmd);
    }, command);
    return index;
  }).toBeGreaterThanOrEqual(0);
  return index;
}

async function findBulkInstallCallAfter(page: Page, command: string, afterIndex: number) {
  let index = -1;
  await expect.poll(async () => {
    index = await page.evaluate(({ cmd, after }) => {
      const calls = (window as any).__bulkInstallCalls as Array<{ command: string }>;
      return calls.findIndex((call, i) => i > after && call.command === cmd);
    }, { cmd: command, after: afterIndex });
    return index;
  }).toBeGreaterThanOrEqual(0);
  return index;
}

async function resolveBulkInstallCall(page: Page, index: number, value: unknown) {
  await page.evaluate(({ index, value }) => (window as any).__resolveBulkInstall(index, value), { index, value });
}

async function openBrowseAndWait(page: Page) {
  await page.goto('/');
  await page.getByRole('button', { name: 'Browse', exact: true }).click();
  await expect(page.getByText('Bulk Mod A')).toBeVisible({ timeout: 5000 });
}

test.describe('E3 — Bulk select and install', () => {

  test('clicking cards toggles selection and the install bar reflects it', async ({ page }) => {
    await installBulkSelectMock(page);
    await openBrowseAndWait(page);

    const cardA = bulkCard(page, 'Bulk Mod A');
    const cardB = bulkCard(page, 'Bulk Mod B');

    await cardA.click();
    await expect(page.getByTestId('browse-selection-count')).toHaveText('1 selected');
    await expect(cardA).toHaveClass(/browse-card--selected/);

    await cardB.click();
    await expect(page.getByTestId('browse-selection-count')).toHaveText('2 selected');
    await expect(page.getByRole('button', { name: 'Install 2', exact: true })).toBeVisible();

    // Clicking a selected card unselects it
    await cardA.click();
    await expect(page.getByTestId('browse-selection-count')).toHaveText('1 selected');
    await expect(cardA).not.toHaveClass(/browse-card--selected/);

    // Clear removes the whole selection and the bar
    await page.getByRole('button', { name: 'Clear', exact: true }).click();
    await expect(page.getByTestId('browse-selection-count')).toHaveCount(0);
  });

  test('installed items are blocked from bulk selection', async ({ page }) => {
    await installBulkSelectMock(page);
    await openBrowseAndWait(page);

    await page.locator('#browse-instance-context').selectOption('fabric-121');
    await expect(page.getByText('Installed Mod')).toBeVisible();

    const card = bulkCard(page, 'Installed Mod');
    await expect(card.locator('.browse-context-label--installed')).toBeVisible();

    await card.click();
    await expect(page.getByTestId('browse-selection-count')).toHaveCount(0);
    await expect(card).not.toHaveClass(/browse-card--selected/);
    await expect(page.getByText(/Installed Mod is already installed in My Fabric World/)).toBeVisible();
  });

  test('bulk install with an instance context installs directly to that instance', async ({ page }) => {
    await installBulkSelectMock(page);
    await openBrowseAndWait(page);

    await page.locator('#browse-instance-context').selectOption('fabric-121');
    await expect(page.getByText('Bulk Mod A')).toBeVisible();

    await bulkCard(page, 'Bulk Mod A').click();
    await bulkCard(page, 'Bulk Mod B').click();
    await bulkCard(page, 'Bulk Modrinth').click();
    await page.getByRole('button', { name: 'Install 3 to My Fabric World', exact: true }).click();

    const resolveCall = await findBulkInstallCall(page, 'resolve_install_plan');
    const args = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, resolveCall);
    const intent = args.intent as {
      action: { type: string; items: Array<{ sourceType: string; itemId: string; candidateVersion?: string }> };
      targetInstance: string;
    };
    expect(intent.action.type).toBe('batch-install');
    expect(intent.targetInstance).toBe('fabric-121');
    // Curated items use the registry id; Modrinth-only items use the project id.
    expect(intent.action.items.map((item) => `${item.sourceType}:${item.itemId}`).sort()).toEqual([
      'curated:bulk-mod-a',
      'curated:bulk-mod-b',
      'modrinth:mr-project-1',
    ]);
    // No version selection in bulk mode — the resolver picks the latest compatible.
    expect(intent.action.items.every((item) => item.candidateVersion === undefined)).toBe(true);

    // The install runs in the non-blocking corner card; no focused dialog.
    await expect(page.getByText('Installing 3 selected items')).toBeVisible();
    await expect(page.getByRole('dialog')).toHaveCount(0);

    await resolveBulkInstallCall(page, resolveCall, bulkBatchPlan(args.intent));
    const applyCall = await findBulkInstallCall(page, 'apply_install_plan');
    await resolveBulkInstallCall(page, applyCall, BULK_SUCCESS_OUTCOME);
    await expect(page.getByText('Installation complete', { exact: true })).toBeVisible();

    // Starting the background install clears the selection and the bar.
    await expect(page.getByTestId('browse-selection-count')).toHaveCount(0);
  });

  test('clean bulk plans open confirmation when auto-confirm is disabled', async ({ page }) => {
    await installBulkSelectMock(page, false);
    await openBrowseAndWait(page);

    await page.locator('#browse-instance-context').selectOption('fabric-121');
    await bulkCard(page, 'Bulk Mod A').click();
    await page.getByRole('button', { name: 'Install 1 to My Fabric World', exact: true }).click();

    const resolveCall = await findBulkInstallCall(page, 'resolve_install_plan');
    const args = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, resolveCall);
    await resolveBulkInstallCall(page, resolveCall, bulkBatchPlan(args.intent));

    const review = page.getByRole('dialog');
    await expect(review).toBeVisible();
    await expect(review.getByRole('button', { name: 'Install Batch', exact: true })).toBeVisible();
    await expect(page.getByText('Installing 1 selected item')).toHaveCount(0);

    await review.getByRole('button', { name: 'Install Batch', exact: true }).click();
    const applyCall = await findBulkInstallCall(page, 'apply_install_plan');
    await resolveBulkInstallCall(page, applyCall, BULK_SUCCESS_OUTCOME);
    await expect(page.getByText('Installation complete', { exact: true })).toBeVisible();
  });

  test('always auto-confirm skips dependency details when enabled', async ({ page }) => {
    await installBulkSelectMock(page, true, true);
    await openBrowseAndWait(page);

    await page.locator('#browse-instance-context').selectOption('fabric-121');
    await bulkCard(page, 'Bulk Mod A').click();
    await bulkCard(page, 'Bulk Mod B').click();
    await page.getByRole('button', { name: 'Install 2 to My Fabric World', exact: true }).click();

    const resolveCall = await findBulkInstallCall(page, 'resolve_install_plan');
    const args = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, resolveCall);
    await resolveBulkInstallCall(page, resolveCall, bulkBatchPlanWithOptionalDep(args.intent));

    await expect(page.getByRole('dialog')).toHaveCount(0);
    const applyCall = await findBulkInstallCall(page, 'apply_install_plan');
    await resolveBulkInstallCall(page, applyCall, BULK_SUCCESS_OUTCOME);
    await expect(page.getByText('Installation complete', { exact: true })).toBeVisible();
  });

  test('bulk install without an instance context opens an instance picker', async ({ page }) => {
    await installBulkSelectMock(page);
    await openBrowseAndWait(page);

    await bulkCard(page, 'Bulk Mod A').click();
    await bulkCard(page, 'Bulk Mod B').click();
    await page.getByRole('button', { name: 'Install 2', exact: true }).click();

    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText('Install 2 selected items')).toBeVisible();

    // No instance is pre-selected — an explicit choice is required.
    await expect(dialog.getByRole('button', { name: 'Install 2 items', exact: true })).toBeDisabled();
    await dialog.getByLabel('Target instance').selectOption('fabric-121');
    await dialog.getByRole('button', { name: 'Install 2 items', exact: true }).click();

    const resolveCall = await findBulkInstallCall(page, 'resolve_install_plan');
    const args = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, resolveCall);
    const intent = args.intent as { action: { type: string }; targetInstance: string };
    expect(intent.action.type).toBe('batch-install');
    expect(intent.targetInstance).toBe('fabric-121');

    await expect(page.getByText('Installing 2 selected items')).toBeVisible();
    await resolveBulkInstallCall(page, resolveCall, bulkBatchPlan(args.intent));
    const applyCall = await findBulkInstallCall(page, 'apply_install_plan');
    await resolveBulkInstallCall(page, applyCall, BULK_SUCCESS_OUTCOME);
    await expect(page.getByText('Installation complete', { exact: true })).toBeVisible();
  });

  test('optional dependencies open a focused review with names, links, and unchecked defaults', async ({ page }) => {
    await installBulkSelectMock(page);
    await openBrowseAndWait(page);

    await page.locator('#browse-instance-context').selectOption('fabric-121');
    await expect(page.getByText('Bulk Mod A')).toBeVisible();

    await bulkCard(page, 'Bulk Mod A').click();
    await bulkCard(page, 'Bulk Mod B').click();
    await page.getByRole('button', { name: 'Install 2 to My Fabric World', exact: true }).click();

    const resolveCall = await findBulkInstallCall(page, 'resolve_install_plan');
    const args = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, resolveCall);
    await resolveBulkInstallCall(page, resolveCall, bulkBatchPlanWithOptionalDep(args.intent));

    // The plan needs a decision, so the focused review opens with real names
    // and page links instead of raw ids, and the corner card is dismissed.
    const review = page.getByRole('dialog');
    await expect(review).toBeVisible();
    await expect(page.getByText('Installing 2 selected items')).toHaveCount(0);
    await expect(review.getByText('TerraBlender')).toBeVisible();
    await expect(review.getByText('Required Dependency')).toBeVisible();
    // Deps render in plan order (required-dep first, optional-dep-b second).
    await expect(review.getByRole('link', { name: 'View mod page ↗' })).toHaveCount(2);
    await expect(review.getByRole('link', { name: 'View mod page ↗' }).nth(1))
      .toHaveAttribute('href', 'https://modrinth.com/mod/terrablender');
    // A dependency satisfied by another batch item is labelled as such.
    await expect(review.getByText('Bulk Mod B')).toBeVisible();
    await expect(review.getByText('included in this batch')).toBeVisible();

    // Optional dependencies default to unchecked for bulk installs.
    const optionalCheckbox = review.getByRole('checkbox', { name: 'Include optional dependency TerraBlender' });
    await expect(optionalCheckbox).not.toBeChecked();

    // Opt in, recheck, and review the final plan with new files highlighted.
    await optionalCheckbox.check();
    await review.getByRole('button', { name: 'Install Batch', exact: true }).click();

    const replanCall = await findBulkInstallCallAfter(page, 'resolve_install_plan', resolveCall);
    const replanArgs = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, replanCall);
    const replanIntent = replanArgs.intent as {
      optionalDeps: { type: string; deps: string[] };
      action: { type: string };
    };
    expect(replanIntent.action.type).toBe('batch-install');
    expect(replanIntent.optionalDeps).toEqual({ type: 'include', deps: ['optional-dep-b'] });

    await resolveBulkInstallCall(page, replanCall, bulkBatchPlanWithNewOptionalFile(replanArgs.intent));
    const finalReview = page.getByRole('dialog');
    await expect(finalReview.getByText('New options have been checked')).toBeVisible();
    await expect(finalReview.getByText('New Items From Optional Dependencies')).toBeVisible();
    await expect(finalReview.getByText('+ optional-dep-b.jar')).toBeVisible();
    await finalReview.getByRole('button', { name: 'Install Batch', exact: true }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    const applyCall = await findBulkInstallCall(page, 'apply_install_plan');
    await resolveBulkInstallCall(page, applyCall, BULK_SUCCESS_OUTCOME);
    await expect(page.getByText('Installation complete', { exact: true })).toBeVisible();
  });

  test('blocking plan errors open the focused review instead of the corner card', async ({ page }) => {
    await installBulkSelectMock(page);
    await openBrowseAndWait(page);

    await page.locator('#browse-instance-context').selectOption('fabric-121');
    await expect(page.getByText('Bulk Mod A')).toBeVisible();

    await bulkCard(page, 'Bulk Mod A').click();
    await bulkCard(page, 'Bulk Mod B').click();
    await page.getByRole('button', { name: 'Install 2 to My Fabric World', exact: true }).click();

    const resolveCall = await findBulkInstallCall(page, 'resolve_install_plan');
    const args = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, resolveCall);
    await resolveBulkInstallCall(page, resolveCall, bulkBatchPlanBlocking(args.intent));

    const review = page.getByRole('dialog');
    await expect(review).toBeVisible();
    await expect(review.getByText(/Required dependency missing-dep could not be resolved/)).toBeVisible();
    await expect(review.getByRole('button', { name: 'Retry Resolution', exact: true })).toBeVisible();
    await expect(review.getByRole('button', { name: 'Cannot Apply' })).toBeDisabled();
    await expect(page.getByText('Installing 2 selected items')).toHaveCount(0);
  });

  test('resolution failures open the focused retry screen', async ({ page }) => {
    await installBulkSelectMock(page);
    await openBrowseAndWait(page);

    await page.locator('#browse-instance-context').selectOption('fabric-121');
    await bulkCard(page, 'Bulk Mod A').click();
    await page.getByRole('button', { name: 'Install 1 to My Fabric World', exact: true }).click();

    const firstResolve = await findBulkInstallCall(page, 'resolve_install_plan');
    await page.evaluate((index) => {
      (window as any).__rejectBulkInstall(index, {
        code: 'ERR_VERSION_NOT_FOUND',
        message: "No compatible version found for Modrinth item 'bulk-mod-a' on Minecraft 1.21.1 / fabric. Closest available: 1.0.0 (bulk-mod-a.jar).",
      });
    }, firstResolve);

    const retryResolve = await findBulkInstallCallAfter(page, 'resolve_install_plan', firstResolve);
    await page.evaluate((index) => {
      (window as any).__rejectBulkInstall(index, {
        code: 'ERR_VERSION_NOT_FOUND',
        message: "No compatible version found for Modrinth item 'bulk-mod-a' on Minecraft 1.21.1 / fabric. Closest available: 1.0.0 (bulk-mod-a.jar).",
      });
    }, retryResolve);

    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText(/Closest available: 1\.0\.0/)).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Retry' })).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Try Closest Version' })).toBeVisible();
    await dialog.getByRole('button', { name: 'Try Closest Version' }).click();
    const closestResolve = await findBulkInstallCallAfter(page, 'resolve_install_plan', retryResolve);
    const closestArgs = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, closestResolve);
    expect((closestArgs.intent as any).overrides.allowClosestVersion).toBe(true);
    await page.evaluate((index) => {
      (window as any).__rejectBulkInstall(index, {
        code: 'ERR_VERSION_NOT_FOUND',
        message: "No compatible version found for Modrinth item 'bulk-mod-a' on Minecraft 1.21.1 / fabric. Closest available: 1.0.0 (bulk-mod-a.jar).",
      });
    }, closestResolve);
    await expect(dialog.getByRole('button', { name: 'Skip This Mod' })).toBeVisible();
    await dialog.getByRole('button', { name: 'Skip This Mod' }).click();
    const skipResolve = await findBulkInstallCallAfter(page, 'resolve_install_plan', closestResolve);
    const skipArgs = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, skipResolve);
    expect((skipArgs.intent as any).overrides.skipItems).toEqual(['bulk-mod-a']);
    await resolveBulkInstallCall(page, skipResolve, bulkBatchPlan(skipArgs.intent));
    const applyCall = await findBulkInstallCall(page, 'apply_install_plan');
    await resolveBulkInstallCall(page, applyCall, BULK_SUCCESS_OUTCOME);
    await expect(page.getByText('Installation complete', { exact: true })).toBeVisible();
  });

  test('background hash failures open retry and skip actions', async ({ page }) => {
    await installBulkSelectMock(page);
    await openBrowseAndWait(page);

    await page.locator('#browse-instance-context').selectOption('fabric-121');
    await bulkCard(page, 'Bulk Mod A').click();
    await page.getByRole('button', { name: 'Install 1 to My Fabric World', exact: true }).click();

    const resolveCall = await findBulkInstallCall(page, 'resolve_install_plan');
    const args = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, resolveCall);
    await resolveBulkInstallCall(page, resolveCall, bulkBatchPlanWithRootArtifact(args.intent));

    const applyCall = await findBulkInstallCall(page, 'apply_install_plan');
    await resolveBulkInstallCall(page, applyCall, {
      type: 'failed',
      error: 'verification failed for bulk-mod-a-1.0.0.jar: Sha256 hash mismatch',
      rollbackPerformed: false,
      snapshotId: null,
    });

    const dialog = page.getByRole('dialog');
    await expect(dialog.getByText(/Sha256 hash mismatch/)).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Retry' })).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Skip This Mod' })).toBeVisible();
    await dialog.getByRole('button', { name: 'Skip This Mod' }).click();

    const retryResolve = await findBulkInstallCallAfter(page, 'resolve_install_plan', applyCall);
    const retryArgs = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, retryResolve);
    expect((retryArgs.intent as any).overrides.skipItems).toEqual(['bulk-mod-a']);
  });

  test('hash-blocked review plans offer skip for the failing batch root', async ({ page }) => {
    await installBulkSelectMock(page);
    await openBrowseAndWait(page);

    await page.locator('#browse-instance-context').selectOption('fabric-121');
    await bulkCard(page, 'Bulk Mod A').click();
    await page.getByRole('button', { name: 'Install 1 to My Fabric World', exact: true }).click();

    const resolveCall = await findBulkInstallCall(page, 'resolve_install_plan');
    const args = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, resolveCall);
    await resolveBulkInstallCall(page, resolveCall, bulkBatchPlanHashBlocking(args.intent));

    const dialog = page.getByRole('dialog');
    await expect(dialog.getByText(/no acceptable published hash/)).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Skip This Mod' })).toBeVisible();
    await dialog.getByRole('button', { name: 'Skip This Mod' }).click();

    const retryResolve = await findBulkInstallCallAfter(page, 'resolve_install_plan', resolveCall);
    const retryArgs = await page.evaluate((index) => (window as any).__bulkInstallCalls[index].args, retryResolve);
    expect((retryArgs.intent as any).overrides.skipItems).toEqual(['bulk-mod-a']);
  });

  test('View Details opens details without toggling selection', async ({ page }) => {
    await installBulkSelectMock(page);
    await openBrowseAndWait(page);

    await bulkCard(page, 'Bulk Mod A').click();
    await expect(page.getByTestId('browse-selection-count')).toHaveText('1 selected');

    // Opening details must not flip selection state
    await page.getByRole('button', { name: 'View Details', exact: true }).first().click();
    await expect(page.getByRole('heading', { name: 'Bulk Mod A' })).toBeVisible({ timeout: 5000 });

    await page.getByRole('button', { name: '← Back' }).click();
    await expect(page.getByTestId('browse-selection-count')).toHaveText('1 selected');
    await expect(bulkCard(page, 'Bulk Mod B')).not.toHaveClass(/browse-card--selected/);
  });

});

// ---------------------------------------------------------------------------
// Multi-page accumulation.
//
// The other pagination tests hand-resolve a single load-more call or return
// `hasMore: false`, so none of them actually prove the list GROWS across
// several pages. This walks three real pages and asserts accumulation.
// ---------------------------------------------------------------------------

const page0 = () => Array.from({ length: 20 }, (_, i) => item(`p0-${i}`, `Page0 Item ${i}`));
const pageN = (n: number) =>
  Array.from({ length: 20 }, (_, i) => item(`p${n}-${i}`, `Page${n} Item ${i}`));

test('infinite scroll accumulates across multiple pages', async ({ page }) => {
  await installBrowseMock(page);
  const initial = await openBrowse(page);
  await resolveCall(page, initial, { items: page0(), total: 60, page: 0, hasMore: true });

  await expect(page.getByText('Page0 Item 0')).toBeVisible();

  // Page 1
  await page.getByTestId('browse-load-sentinel').scrollIntoViewIfNeeded();
  const load1 = await findCall(page, 'browse_load_more');
  expect(await callArg(page, load1, 'pageIndex')).toBe(1);
  await resolveCall(page, load1, { items: pageN(1), total: 60, page: 1, hasMore: true });
  await expect(page.getByText('Page1 Item 0')).toBeVisible();
  await expect(page.getByText('Page0 Item 0')).toBeVisible();

  // Page 2 — the step that matters: does it ask for the NEXT page, or stall?
  await page.getByTestId('browse-load-sentinel').scrollIntoViewIfNeeded();
  const load2 = await findCall(page, 'browse_load_more', undefined, [load1]);
  expect(await callArg(page, load2, 'pageIndex')).toBe(2);
  await resolveCall(page, load2, { items: pageN(2), total: 60, page: 2, hasMore: false });
  await expect(page.getByText('Page2 Item 0')).toBeVisible();

  await expect(page.getByText('All results loaded')).toBeVisible();
});

// ---------------------------------------------------------------------------
// Returning from a mod detail page must restore the loaded pages, not refetch
// page 0. Without this the user lands back at the top of a one-page list after
// having scrolled through several.
// ---------------------------------------------------------------------------

test('returning from mod details keeps the loaded pages', async ({ page }) => {
  await installBrowseMock(page);
  const initial = await openBrowse(page);
  await resolveCall(page, initial, { items: page0(), total: 40, page: 0, hasMore: true });

  // Load a second page so there is state worth preserving.
  await page.getByTestId('browse-load-sentinel').scrollIntoViewIfNeeded();
  const load1 = await findCall(page, 'browse_load_more');
  await resolveCall(page, load1, { items: pageN(1), total: 40, page: 1, hasMore: false });
  await expect(page.getByText('Page1 Item 0')).toBeVisible();

  const callsBefore = await page.evaluate(() => (window as any).__browseCalls.length);

  // Into a detail page and back out, using the app's own controls.
  await page.getByRole('button', { name: 'View Details', exact: true }).first().click();
  await expect(page.getByRole('button', { name: '← Back' })).toBeVisible();
  await page.getByRole('button', { name: '← Back' }).click();

  // Both pages are still present...
  await expect(page.getByText('Page0 Item 0')).toBeVisible();
  await expect(page.getByText('Page1 Item 0')).toBeVisible();
  // ...and no fresh browse_search was issued to rebuild them.
  const callsAfter = await page.evaluate(() => (window as any).__browseCalls.length);
  expect(callsAfter).toBe(callsBefore);
});
