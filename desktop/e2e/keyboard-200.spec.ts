import { test, expect, type Page } from '@playwright/test';

/**
 * Keyboard-only use at 200% text size.
 *
 * Every check here drives real key presses and real clicks, because the faults
 * this guards against (focus left on <body>, a footer scrolled out of reach, a
 * control hidden under another) are invisible to a synthesised event.
 */

const INSTANCE = { instance_id: 'a', name: 'Test Instance', minecraft_version: '1.21', loader: 'fabric', loader_version: '0.16.9', is_locked: false, last_launched_at: '2026-07-12T10:00:00Z' };
const DETAIL = {
  row: { ...INSTANCE, is_modpack: false, jvm_memory_mb: 4096, jvm_gc: 'G1GC', jvm_custom_args: '', created_at: '2026-06-01T00:00:00Z' },
  pack_gaps: [],
  manifest: {
    instance_id: 'a', name: 'Test Instance', created_from_pack: null, minecraft_version: '1.21', loader: 'fabric',
    loader_version: '0.16.9', is_locked: false, mods: [], resourcepacks: [], shaders: [], datapacks: [], worlds: [],
    user_preferences: { java_memory_gb: 4 },
  },
};

const SHA = 'a'.repeat(64);
const FILE = (i: number) => `some-very-long-mod-filename-for-overflow-checks-fabric-0.6.1${i}+mc1.21.1-build.${i}.jar`;
const CONTENT = ['Sodium', 'Lithium', 'Iris Shaders Companion Extended Edition'].map((name, i) => ({
  key: `mod:${FILE(i)}:${SHA}`, filename: FILE(i), display_name: name, version: `0.6.1${i}`, content_type: 'mod',
  enabled: true, installed_at: '2026-07-01T00:00:00Z', source: 'modrinth', source_label: 'Modrinth', source_url: null,
  registry_id: null, modrinth_id: `p${i}`, mod_jar_id: `mod-${i}`, loader_mod_id: `mod-${i}`, size_bytes: 123456,
  file_present: true, resolved_path: null, author: 'Some Author', categories: ['Optimization'], icon_url: null,
  curation_status: 'unknown', agora_score: null, modrinth_downloads: 1000, metadata_status: 'unavailable',
}));
const UPDATES = [0, 1, 2].map((i) => ({
  filename: FILE(i), mod_jar_id: `mod-${i}`, current_version: `0.6.1${i}`, latest_version: '0.7.0', target_version: '0.7.0', source: 'modrinth',
}));

function plan() {
  const hash = { values: [{ algorithm: 'sha256', value: SHA }] };
  return {
    fingerprint: 'fp-rich',
    intent: { action: { type: 'batch-update', items: [] }, targetInstance: 'a', optionalDeps: { type: 'prompt' }, requestedBy: 'interactive', overrides: { allowReplace: false, skipHealthScan: false, forceConflictResolution: {} } },
    operation: { type: 'batch-update', operations: [] },
    dependencies: [],
    conflicts: [],
    filesToAdd: [0, 1, 2].map((i) => ({ targetFilename: FILE(i), stagingFilename: `s${i}.jar`, artifact: {}, hashes: hash, size: 250000 })),
    filesToRemove: [], filesToDisable: [],
    snapshot: { label: 'Before updating', estimatedBytes: 500000 },
    diskEstimate: { downloadBytes: 250000, snapshotBytes: 500000, applyOverheadBytes: 100000, peakAdditionalBytes: 600000, postCommitDeltaBytes: 250000 },
    warnings: [{ code: 'w1', message: 'This version was built for a different Minecraft patch release and may behave unexpectedly in your instance.' }],
    blockingErrors: [], pendingChoices: [],
    createdAt: '2026-07-12T17:00:00Z', instanceStateHash: 'abc', registryRevision: 'v1',
  };
}

const LONG = 'A long description that keeps going so that it wraps over several lines when the text is scaled to twice its size, which is when cards used to run into the tags below them. ';
const BROWSE_ITEMS = [0, 1, 2].map((i) => ({
  id: `m${i}`, source: 'curated',
  registryItem: { id: `m${i}`, name: `Mod number ${i}`, content_type: 'mod', download_strategy: 'github_release', upvotes: 0, downvotes: 0, net_score: 0 },
  modrinthResult: null, name: `Mod number ${i} with a fairly long display name`, iconUrl: null,
  description: LONG + LONG, contentType: 'mod', heroImageUrl: null, author: 'Some Author With A Long Name',
  categories: ['optimization', 'performance', 'client-side', 'utility'], downloads: 1234567, follows: 4321,
  upvotes: 3, downvotes: 0, netScore: 3, supportedVersions: ['1.21', '1.20.1', '1.19.2'], sourcePageUrl: 'https://example.com',
}));
const PROVIDER_CATEGORIES = Array.from({ length: 150 }, (_, i) => ({ id: `cat${i}`, label: `Category ${i}`, contentTypes: ['mod'] }));

const REGISTRY_ITEM = {
  id: 'test-mod', name: 'Test Mod', content_type: 'mod', download_strategy: 'github_release', source_identifier: 'test/mod',
  sha256: 'abc123', upvotes: 10, downvotes: 2, net_score: 8, velocity: 1.5, status: 'active', is_immune: false, immunity_reason: null,
  allow_comments: true, icon_url: null, gallery_urls_json: null, date_added: '2026-01-01', compatible_versions_json: null,
  description: LONG + LONG, body_markdown: `${LONG}\n\n\`\`\`\n${'x'.repeat(300)}\n\`\`\`\n\n| a | b |\n|---|---|\n| ${'long '.repeat(40)} | y |\n`,
  page_url: null, license_id: 'MIT', source_updated_at: null, modrinth_id: null,
};

type Route = null | 'instance-detail' | 'mod-detail' | 'browse';

async function boot(page: Page, route: Route = null, fontScale = 2, size = { width: 1280, height: 800 }) {
  await page.setViewportSize(size);
  await page.addInitScript(({ route, fontScale, instance, detail, content, updates, plan, browseItems, providerCategories, registryItem }) => {
    localStorage.setItem('agora-ui-preferences', JSON.stringify({ version: 1, fontScale }));
    if (route === 'instance-detail') window.history.replaceState({ __agora: { type: 'instance-detail', instanceId: 'a' } }, '', '');
    if (route === 'mod-detail') window.history.replaceState({ __agora: { type: 'mod-detail', itemId: 'test-mod', browseInstanceId: 'a' } }, '', '');
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: {
        transformCallback() { return 1; },
        unregisterCallback() {},
        invoke(command: string, args: Record<string, unknown> = {}) {
          if (command === 'get_setting') {
            if (args.key === 'onboarding_complete' || args.key === 'modrinth_enabled') return Promise.resolve(true);
            return Promise.resolve(false);
          }
          if (command === 'get_registry_status') return Promise.resolve({ has_cached_db: true, cached_tag: 't', cached_schema_version: 5, latest_tag: 't', update_available: false, checked: true, message: 'Catalog ready.' });
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);
          if (command === 'list_instances') return Promise.resolve([instance]);
          if (command === 'get_instance_detail') return Promise.resolve(detail);
          if (command === 'list_instance_content') return Promise.resolve(content);
          if (command === 'check_instance_updates') return Promise.resolve(updates);
          if (command === 'resolve_install_plan') return Promise.resolve(plan);
          if (command === 'list_provider_categories') return Promise.resolve([{ providerId: 'modrinth', providerTitle: 'Modrinth', categories: providerCategories }]);
          if (command === 'browse_search' || command === 'browse_load_more') return Promise.resolve({ items: browseItems, total: browseItems.length, page: 0, hasMore: false, hiddenByFilters: null });
          if (command === 'get_registry_item') return Promise.resolve(args.itemId === registryItem.id ? registryItem : null);
          if (command === 'list_mod_versions' || command === 'list_mod_versions_load_more') return Promise.resolve({ items: [0, 1, 2].map((i) => ({ version: `1.${i}.0-beta.${i}+build.12345.with.long.version.string`, filename: `test-mod-1.${i}.0-fabric-1.21.1.jar`, download_url: 'https://example.com/x.jar', mc_version: '1.21.1', loader: 'fabric', release_date: '2026-07-01T00:00:00Z', is_compatible: true, is_prerelease: i === 1 })), hasMore: false, total: 3 });
          if (command === 'get_auth_status') return Promise.resolve(false);
          if (command === 'get_item_vote') return Promise.resolve({ vote: null, conflicted: false });
          if (command === 'list_categories' || command === 'list_manifest_loaders' || command === 'list_manifest_mc_versions') return Promise.resolve([]);
          if (/^(list_|for_you|search_|query_launch_state)/.test(command)) return Promise.resolve([]);
          return Promise.resolve(null);
        },
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  }, {
    route, fontScale, instance: INSTANCE, detail: DETAIL, content: CONTENT, updates: UPDATES, plan: plan(),
    browseItems: BROWSE_ITEMS, providerCategories: PROVIDER_CATEGORIES, registryItem: REGISTRY_ITEM,
  });
  await page.goto('/');
}

async function noSidewaysScroll(page: Page) {
  const { scrollWidth, clientWidth } = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
  }));
  expect(scrollWidth).toBeLessThanOrEqual(clientWidth);
}

/** Opens the review dialog from the instance editor with the keyboard. */
async function openReview(page: Page, size = { width: 1280, height: 800 }) {
  await boot(page, 'instance-detail', 2, size);
  await expect(page.getByRole('heading', { name: /Installed Mods/ })).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: 'Check for updates' }).click();
  const updateAll = page.getByRole('button', { name: /^Update all/ });
  await expect(updateAll).toBeVisible();
  await updateAll.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog', { name: 'Review Instance Changes' })).toBeVisible();
}

test('review dialog takes focus, traps Tab, closes on Escape and returns focus (200% text)', async ({ page }) => {
  await openReview(page);
  const dialog = page.getByRole('dialog', { name: 'Review Instance Changes' });
  // Focus is inside the dialog without any key press.
  await expect.poll(() => page.evaluate(() => !!document.activeElement?.closest('[role="dialog"]'))).toBe(true);
  // 30 Tab presses never leave it.
  for (let i = 0; i < 30; i += 1) {
    await page.keyboard.press('Tab');
    const inside = await page.evaluate(() => !!document.activeElement?.closest('[role="dialog"][aria-modal="true"]'));
    expect(inside, `Tab #${i + 1} left the dialog`).toBe(true);
  }
  // Shift+Tab wraps backwards too.
  for (let i = 0; i < 30; i += 1) {
    await page.keyboard.press('Shift+Tab');
    const inside = await page.evaluate(() => !!document.activeElement?.closest('[role="dialog"][aria-modal="true"]'));
    expect(inside, `Shift+Tab #${i + 1} left the dialog`).toBe(true);
  }
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^Update all/ })).toBeFocused();
});

test('review dialog: Cancel and Install are in view and Install takes a real click (200% text)', async ({ page }) => {
  await openReview(page);
  const dialog = page.getByRole('dialog', { name: 'Review Instance Changes' });
  const install = dialog.getByRole('button', { name: 'Apply Updates' });
  const cancel = dialog.getByRole('button', { name: 'Cancel' });
  await expect(install).toBeVisible();
  const vh = page.viewportSize()!.height;
  for (const button of [install, cancel]) {
    const box = (await button.boundingBox())!;
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.y + box.height).toBeLessThanOrEqual(vh);
  }
  const dialogBox = (await dialog.boundingBox())!;
  expect(dialogBox.y + dialogBox.height).toBeLessThanOrEqual(vh);
  // Reached with the keyboard alone.
  await install.focus();
  await expect(install).toBeFocused();
  await cancel.click({ timeout: 3_000 });
  await expect(dialog).toHaveCount(0);
});

test('review dialog at 200% in a short window keeps its footer and scrolls the body', async ({ page }) => {
  await openReview(page, { width: 1024, height: 560 });
  const dialog = page.getByRole('dialog', { name: 'Review Instance Changes' });
  const install = dialog.getByRole('button', { name: 'Apply Updates' });
  await expect(install).toBeVisible();
  const vh = page.viewportSize()!.height;
  const box = (await install.boundingBox())!;
  expect(box.y + box.height).toBeLessThanOrEqual(vh);
  const scrollsInBody = await dialog.evaluate((el) => {
    const body = el.querySelector('[data-install-review-body]') as HTMLElement;
    return { dialogScrolls: el.scrollHeight > el.clientHeight + 1, bodyScrolls: body.scrollHeight > body.clientHeight + 1 };
  });
  expect(scrollsInBody.dialogScrolls).toBe(false);
  expect(scrollsInBody.bodyScrolls).toBe(true);
  await install.click({ timeout: 3_000 });
});

test('Ctrl+K opens the palette from a slider, checkbox and radio but not from a text field', async ({ page }) => {
  await boot(page, null, 1);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.getByRole('tab', { name: /Appearance/ }).click();
  await page.getByRole('tab', { name: 'Interface' }).click();
  const slider = page.getByRole('slider', { name: 'Text scale' });
  await slider.focus();
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  // Other inputs that take no typing.
  const box = page.locator('input[type="checkbox"], input[type="radio"]').first();
  if (await box.count()) {
    await box.focus();
    await page.keyboard.press('Control+k');
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
  }
  // A text field keeps its own Ctrl+K.
  await page.getByRole('button', { name: 'Browse', exact: true }).first().click();
  await page.getByRole('searchbox').or(page.getByRole('textbox').first()).first().focus();
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('Browse: the category chips cost one Tab stop, then the arrows move through them', async ({ page }) => {
  await boot(page, null, 1);
  await page.getByRole('button', { name: 'Browse', exact: true }).first().click();
  const group = page.getByRole('radiogroup', { name: 'Source categories' });
  await expect(group).toBeVisible();
  await expect(group.getByRole('radio')).toHaveCount(151);
  const stops = await group.getByRole('radio').evaluateAll((els) => els.filter((el) => (el as HTMLElement).tabIndex >= 0).length);
  expect(stops).toBe(1);
  await group.getByRole('radio', { name: 'All' }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(group.getByRole('radio', { name: 'Category 0' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(group.getByRole('radio', { name: 'Category 0' })).toHaveAttribute('aria-checked', 'true');
  // One Tab leaves the group.
  await page.keyboard.press('Tab');
  const stillInGroup = await page.evaluate(() => !!document.activeElement?.closest('[role="radiogroup"][aria-label="Source categories"]'));
  expect(stillInGroup).toBe(false);
});

test('Settings: arrows move between categories, Tab moves on to the page', async ({ page }) => {
  await boot(page, null, 2);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  const tabs = page.getByRole('tablist', { name: 'Settings sections' }).getByRole('tab');
  if (process.env.A11Y_SHOTS) await page.screenshot({ path: `${process.env.A11Y_SHOTS}/settings.png` });
  await tabs.first().focus();
  await page.keyboard.press('ArrowDown');
  await expect(tabs.nth(1)).toBeFocused();
  await expect(tabs.nth(1)).toHaveAttribute('aria-selected', 'true');
  // Every category name is shown whole: no ellipsis, nothing wider than its box.
  const clipped = await tabs.evaluateAll((els) => els.filter((el) => {
    const label = el.querySelector('span.font-medium') as HTMLElement | null;
    const style = label ? getComputedStyle(label) : null;
    return !label || style!.textOverflow === 'ellipsis' || label.scrollWidth > label.clientWidth + 1 || el.scrollWidth > el.clientWidth + 1;
  }).map((el) => el.textContent));
  expect(clipped).toEqual([]);
});

test('Text scale shows its value and announces it', async ({ page }) => {
  await boot(page, null, 2);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.getByRole('tab', { name: /Appearance/ }).click();
  await page.getByRole('tab', { name: 'Interface' }).click();
  const slider = page.getByRole('slider', { name: 'Text scale' });
  await expect(slider).toHaveAttribute('aria-valuetext', '200%');
  await expect(slider.locator('xpath=ancestor::label')).toContainText('200%');
});

test('no word breaks across lines in the sidebar at 200% text', async ({ page }) => {
  await boot(page, null, 2);
  const broken = await page.evaluate(() => {
    const out: string[] = [];
    const walker = document.createTreeWalker(document.querySelector('[data-testid="sidebar"]')!, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const text = node.textContent ?? '';
      for (const match of text.matchAll(/[A-Za-z]{4,}/g)) {
        const range = document.createRange();
        range.setStart(node, match.index!);
        range.setEnd(node, match.index! + match[0].length);
        const lines = new Set([...range.getClientRects()].map((r) => Math.round(r.top / 4)));
        if (lines.size > 1) out.push(match[0]);
      }
    }
    return out;
  });
  expect(broken).toEqual([]);
});

test('Browse result cards: text does not collide at 200% text', async ({ page }) => {
  await boot(page, null, 2);
  await page.getByRole('button', { name: 'Browse', exact: true }).first().click();
  await expect(page.getByRole('heading', { name: /Mod number 0/ })).toBeVisible();
  if (process.env.A11Y_SHOTS) { await page.getByRole('heading', { name: /Mod number 0/ }).scrollIntoViewIfNeeded(); await page.screenshot({ path: `${process.env.A11Y_SHOTS}/browse.png` }); }
  const collisions = await page.evaluate(() => {
    const out: string[] = [];
    for (const card of document.querySelectorAll('.browse-list-card, .browse-tile-card')) {
      const parts = [...card.querySelectorAll('.browse-card-title, .browse-card-source, [class*="__description"], .browse-category-list, .browse-list-card__inline-stats, [class*="browse-card-versions"], .browse-recommendation')]
        .map((el) => ({ el, r: el.getBoundingClientRect() }))
        .filter(({ r }) => r.width > 0 && r.height > 0);
      for (let i = 0; i < parts.length; i += 1) {
        for (let j = i + 1; j < parts.length; j += 1) {
          const a = parts[i].r; const b = parts[j].r;
          const w = Math.min(a.right, b.right) - Math.max(a.left, b.left);
          const h = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
          if (w > 2 && h > 2) out.push(`${parts[i].el.className} x ${parts[j].el.className}`);
        }
      }
      // A line box shorter than its own text is what makes lines print over each other.
      for (const el of card.querySelectorAll('p, span, h3')) {
        if (!el.textContent?.trim() || el.children.length > 0) continue;
        const cs = getComputedStyle(el);
        const lh = parseFloat(cs.lineHeight);
        if (!Number.isNaN(lh) && lh < parseFloat(cs.fontSize) * 1.05) out.push(`line-height ${lh} < font-size ${cs.fontSize} on ${(el as HTMLElement).className}`);
        if (cs.textOverflow === 'ellipsis' && el.scrollWidth > el.clientWidth + 1 && !(el as HTMLElement).className.includes('browse-card-source')) out.push(`ellipsis on ${(el as HTMLElement).className}`);
      }
      // Text may not spill out of the card either.
      const c = card.getBoundingClientRect();
      for (const { el, r } of parts) if (r.right > c.right + 1 || r.bottom > c.bottom + 1) out.push(`${el.className} spills out of its card`);
    }
    return out;
  });
  expect(collisions).toEqual([]);
  await noSidewaysScroll(page);
});

test('mod detail page does not scroll sideways at 200% text', async ({ page }) => {
  await boot(page, 'mod-detail', 2);
  await expect(page.getByRole('heading', { name: 'Test Mod' })).toBeVisible({ timeout: 15_000 });
  for (const [i, name] of ['About', 'Gallery', 'Versions', 'Agora'].entries()) {
    await page.getByRole('button', { name, exact: true }).click();
    await page.waitForTimeout(150);
    if (process.env.A11Y_SHOTS) { await page.locator('main').evaluate((m) => { m.scrollTop = 600; }); await page.screenshot({ path: `${process.env.A11Y_SHOTS}/md-${name}.png` }); await page.locator('main').evaluate((m) => { m.scrollTop = 0; }); }
    await noSidewaysScroll(page);
    // Neither the window nor any scroll box that holds the page may scroll sideways
    // (a code block scrolling inside itself is fine).
    const scrollers = await page.evaluate(() => {
      const out: string[] = [];
      for (const el of document.querySelectorAll('main, main *')) {
        if (el.closest('pre, code')) continue;
        const style = getComputedStyle(el);
        if (style.overflowX !== 'visible' && el.scrollWidth > el.clientWidth + 1) out.push(`${el.tagName}.${(el as HTMLElement).className}: ${el.scrollWidth} > ${el.clientWidth}`);
      }
      return out;
    });
    expect(scrollers, `tab ${i}`).toEqual([]);
    // Dates and badges are not split across lines.
    const split = await page.evaluate(() => [...document.querySelectorAll('main td, main span')]
      .filter((el) => el.children.length === 0 && /^(\d{4}-\d{2}-\d{2}|beta)$/i.test(el.textContent?.trim() ?? ''))
      .filter((el) => { const range = document.createRange(); range.selectNodeContents(el); return new Set([...range.getClientRects()].map((r) => Math.round(r.top / 4))).size > 1; })
      .map((el) => el.textContent));
    expect(split, `tab ${i}`).toEqual([]);
    // Nothing visible pokes out past the page's right edge.
    const poking = await page.evaluate(() => {
      const edge = document.querySelector('main')!.getBoundingClientRect().right + 1;
      return [...document.querySelectorAll('main *')].filter((el) => !el.closest('pre') && el.getBoundingClientRect().width > 0 && el.getBoundingClientRect().right > edge)
        .slice(0, 5).map((el) => `${el.tagName}.${(el as HTMLElement).className}`);
    });
    expect(poking, `tab ${i}`).toEqual([]);
  }
});

test('Browse: "Skip to results" reaches the results in a couple of stops, and shows where focus landed', async ({ page }) => {
  await boot(page, null, 1);
  await page.getByRole('button', { name: 'Browse', exact: true }).first().click();
  await expect(page.getByRole('heading', { name: /Mod number 0/ })).toBeVisible();

  const skip = page.getByRole('button', { name: 'Skip to results' });
  let presses = 0;
  while (!(await skip.evaluate((el) => el === document.activeElement)) && presses < 6) {
    await page.keyboard.press('Tab');
    presses += 1;
  }
  await expect(skip).toBeFocused();
  expect(presses).toBeLessThanOrEqual(3);

  await page.keyboard.press('Enter');
  const region = page.locator('#browse-results');
  await expect(region).toHaveAttribute('data-skip-landing', '');
  await expect(region).toBeFocused();
  const outline = await region.evaluate((el) => getComputedStyle(el).outlineStyle);
  expect(outline).not.toBe('none');

  // The next Tab is the first result, not a filter.
  await page.keyboard.press('Tab');
  const firstStop = await page.evaluate(() => !!document.activeElement?.closest('#browse-results'));
  expect(firstStop).toBe(true);
});

test('Instance editor: "Skip to mods" jumps past the header and sub-tabs to the mod list', async ({ page }) => {
  await boot(page, 'instance-detail', 1);
  await expect(page.getByRole('heading', { name: /Installed Mods/ })).toBeVisible({ timeout: 15_000 });

  // The skip link is the first control on the page, ahead of Back.
  await page.getByRole('button', { name: '← Back' }).focus();
  await page.keyboard.press('Shift+Tab');
  const skip = page.getByRole('button', { name: 'Skip to mods' });
  await expect(skip).toBeFocused();

  await page.keyboard.press('Enter');
  const landed = await page.evaluate(() => document.activeElement?.hasAttribute('data-skip-landing') ?? false);
  expect(landed).toBe(true);

  // From the list, "Import Mod" is a handful of stops, not the thirty or so it
  // was behind the header and sub-tabs.
  let presses = 0;
  const importMod = page.getByRole('button', { name: 'Import Mod', exact: true });
  while (!(await importMod.evaluate((el) => el === document.activeElement)) && presses < 12) {
    await page.keyboard.press('Tab');
    presses += 1;
  }
  await expect(importMod).toBeFocused();
  expect(presses).toBeLessThanOrEqual(10);
});

test('Ctrl+K finds Text size and opens its Settings page', async ({ page }) => {
  await boot(page, null, 1);
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.keyboard.type('text size');
  await page.keyboard.press('Enter');

  await expect(page.getByRole('slider', { name: 'Text scale' })).toBeVisible();
  await expect(page.getByRole('tab', { name: 'Interface' })).toHaveAttribute('aria-selected', 'true');
});

test('Ctrl+K from an open Settings page moves it to Appearance', async ({ page }) => {
  await boot(page, null, 1);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.getByRole('tab', { name: /General/ }).first().click();
  await page.keyboard.press('Control+k');
  await page.keyboard.type('appearance');
  await page.keyboard.press('Enter');

  await expect(page.getByRole('tab', { name: /Appearance/ })).toHaveAttribute('aria-selected', 'true');
  await expect(page.getByRole('tab', { name: 'Theme' })).toHaveAttribute('aria-selected', 'true');
});
