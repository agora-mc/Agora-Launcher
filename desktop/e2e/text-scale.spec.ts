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
