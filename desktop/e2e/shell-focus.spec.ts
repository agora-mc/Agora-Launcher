import { test, expect, type Page } from '@playwright/test';

/** Mirrors the command palette spec's mock: enough for Home and My Instances to render. */
async function installShellMock(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    const instances = [
      { instance_id: 'instance-one', name: 'Instance One', loader: 'fabric', loader_version: '0.16', minecraft_version: '1.21', is_locked: false, last_launched_at: null },
    ];
    const internals = {
      transformCallback(callback: (...args: unknown[]) => void) {
        const id = ++callbackId;
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback(id: number) { callbacks.delete(id); },
      invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'get_setting') {
          return Promise.resolve(args.key === 'onboarding_complete');
        }
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        if (command === 'list_instances') return Promise.resolve(instances);
        if (command === 'query_launch_state') return Promise.resolve([]);
        if (command.startsWith('list_')) return Promise.resolve([]);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  });
}

/** Tag and text of the element that has focus. */
function activeElement(page: Page) {
  return page.evaluate(() => {
    const el = document.activeElement as HTMLElement | null;
    return { tag: el?.tagName ?? null, text: el?.textContent?.trim() ?? null, inMain: el?.closest('main') != null };
  });
}

test.beforeEach(async ({ page }) => {
  await installShellMock(page);
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Skip to content' })).toHaveCount(1);
});

test('the first load does not move focus into the page', async ({ page }) => {
  const focused = await activeElement(page);
  expect(focused.tag).toBe('BODY');
});

test('Tab reaches "Skip to content" first, and Enter moves focus into the page heading', async ({ page }) => {
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Skip to content' })).toBeFocused();

  await page.keyboard.press('Enter');
  const focused = await activeElement(page);
  expect(focused.inMain).toBe(true);
  expect(focused.tag === 'H1' || focused.tag === 'H2').toBe(true);
});

test('choosing a page in the sidebar moves focus to the new page heading', async ({ page }) => {
  await page.getByRole('button', { name: 'My Instances' }).click();
  await expect(page.getByRole('heading', { name: 'My Instances' })).toBeVisible();

  const focused = await activeElement(page);
  expect(focused.inMain).toBe(true);
  expect(focused.text).toBe('My Instances');
});

test('"Skip to content" leaves a visible focus ring on the heading it lands on', async ({ page }) => {
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Skip to content' })).toBeFocused();
  await page.keyboard.press('Enter');

  const ring = await page.evaluate(() => {
    const el = document.activeElement as HTMLElement;
    const style = getComputedStyle(el);
    return { tag: el.tagName, outlineStyle: style.outlineStyle, outlineWidth: style.outlineWidth };
  });
  expect(ring.tag === 'H1' || ring.tag === 'H2').toBe(true);
  expect(ring.outlineStyle).not.toBe('none');
  expect(parseFloat(ring.outlineWidth)).toBeGreaterThan(0);
});

test('a page change does not leave a ring on the new heading', async ({ page }) => {
  await page.getByRole('button', { name: 'My Instances' }).click();
  await expect(page.getByRole('heading', { name: 'My Instances' })).toBeVisible();

  const outline = await page.evaluate(() => getComputedStyle(document.activeElement as HTMLElement).outlineStyle);
  expect(outline).toBe('none');
});
