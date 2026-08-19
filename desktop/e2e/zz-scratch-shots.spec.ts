import { test, type Page } from '@playwright/test';
import path from 'node:path';

const OUT = 'C:/Users/jarja/AppData/Local/Temp/claude/D--Agora--claude-worktrees-settings-page-tabs-redesign-b04d75/9ac23260-eeba-416a-8dbf-192650b6e759/scratchpad';

async function mock(page: Page) {
  await page.addInitScript(() => {
    const internals = {
      transformCallback() { return 1; },
      unregisterCallback() {},
      invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'get_setting') {
          const key = args.key as string;
          if (key === 'onboarding_complete') return Promise.resolve(true);
          if (key === 'advanced_mode') return Promise.resolve('true');
          if (key === 'modrinth_enabled') return Promise.resolve(true);
          if (key === 'launch_mode') return Promise.resolve('delegation');
          if (key === 'java_runtime_mode') return Promise.resolve('automatic');
          if (key === 'always_pre_touch') return Promise.resolve(true);
          if (key === 'mojang_launcher_path') return Promise.resolve('');
          return Promise.resolve(null);
        }
        if (command === 'plugin:app|version') return Promise.resolve('0.1.0');
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'list_instances' || command === 'list_java_runtimes') return Promise.resolve([]);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  });
}

test('scratch: settings tabs', async ({ page }) => {
  await mock(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.waitForTimeout(500);
  await page.screenshot({ path: path.join(OUT, 'settings-general.png') });

  const rail = page.getByRole('navigation', { name: 'Settings sections' });
  await rail.getByRole('tab', { name: 'Appearance' }).click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: path.join(OUT, 'settings-appearance-theme.png') });

  await page.getByRole('tab', { name: 'Living background' }).click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: path.join(OUT, 'settings-living-background.png') });

  await rail.getByRole('tab', { name: 'Launching' }).click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: path.join(OUT, 'settings-launching.png') });
});

test('scratch: settings narrow', async ({ page }) => {
  await mock(page);
  await page.setViewportSize({ width: 900, height: 800 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.waitForTimeout(500);
  await page.screenshot({ path: path.join(OUT, 'settings-narrow.png') });
});
