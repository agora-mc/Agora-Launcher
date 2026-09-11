import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';

const html = readFileSync(new URL('../../examples/plugins/custom-dashboard/view.html', import.meta.url), 'utf8');

test('custom plugin frame calls its own command and cannot access parent IPC', async ({ page }) => {
  await page.addInitScript(({ html }) => {
    let enabled = true;
    const contribution = { id: 'agora.custom-dashboard/dashboard', pluginId: 'agora.custom-dashboard',
      localId: 'dashboard', kind: 'page', title: 'Custom dashboard' };
    const plugin = { id: 'agora.custom-dashboard', name: 'Custom dashboard prototype', version: '0.1.0',
      license: 'MIT', enabled: true, running: true, development: true, status: { state: 'ready' },
      statusText: 'Ready', capabilities: ['instance:read'], declaredHosts: [], contributions: [contribution], droppedEvents: 0,
      definitions: { pages: [{ id: 'dashboard', title: 'Custom dashboard', view: { kind: 'custom', html: 'view.html' } }],
        commands: [{ id: 'count', title: 'Count', export: 'count', surfaces: [] }], instancePanels: [], diagnostics: [], launchChecks: [] } };
    Object.assign(window, {
      pluginCalls: [] as unknown[],
      __TAURI_INTERNALS__: {
        transformCallback() { return 1; }, unregisterCallback() {},
        invoke(command: string, args: Record<string, unknown> = {}) {
          if (command === 'get_setting') return Promise.resolve(args.key === 'onboarding_complete' ? true : args.key === 'plugins_enabled' ? enabled : null);
          if (command === 'plugins_enabled') return Promise.resolve(enabled);
          if (command === 'start_plugins') return Promise.resolve([]);
          if (command === 'list_plugins') return Promise.resolve([plugin]);
          if (command === 'list_plugin_contributions') return Promise.resolve(enabled ? [contribution] : []);
          if (command === 'read_plugin_custom_view') return Promise.resolve(html);
          if (command === 'run_plugin_command') {
            (window as unknown as { pluginCalls: unknown[] }).pluginCalls.push(args);
            return Promise.resolve({ total: 3 });
          }
          if (command === 'set_setting') { if (args.key === 'plugins_enabled') enabled = args.value === true; return Promise.resolve(null); }
          if (command === 'disable_all_plugins') { enabled = false; return Promise.resolve(1); }
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);
          if (command.startsWith('list_')) return Promise.resolve([]);
          return Promise.resolve(null);
        },
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  }, { html });
  await page.goto('/');
  await page.getByRole('button', { name: 'Custom dashboard', exact: true }).click();
  const panel = page.frameLocator('iframe[title="Custom dashboard"]');
  await panel.getByRole('button', { name: 'Read and remember instance count' }).click();
  await expect(panel.getByRole('status')).toHaveText('You have 3 instances. Count saved.');
  await expect.poll(() => page.evaluate(() => (window as unknown as { pluginCalls: unknown[] }).pluginCalls.length)).toBe(1);
  const child = page.frames().find((frame) => frame.url().startsWith('data:'))!;
  expect(await child.evaluate(() => {
    try { return typeof (parent as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__; }
    catch { return 'blocked'; }
  })).toBe('blocked');
  expect(await child.evaluate(async () => {
    try { await fetch('https://example.com/'); return 'allowed'; } catch { return 'blocked'; }
  })).toBe('blocked');
  await page.evaluate(() => window.postMessage({ type: 'agora:command', requestId: 'forged', commandId: 'count' }, '*'));
  expect(await page.evaluate(() => (window as unknown as { pluginCalls: unknown[] }).pluginCalls.length)).toBe(1);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('tab', { name: 'Services', exact: true }).click();
  await page.getByRole('tab', { name: 'Plugins', exact: true }).click();
  await page.getByRole('switch', { name: 'Enable community plugins' }).click();
  await expect(page.getByRole('button', { name: 'Custom dashboard', exact: true })).toHaveCount(0);
  await page.goBack();
  await expect(page.getByRole('heading', { name: 'This plugin page is unavailable' })).toBeVisible();
  await page.getByRole('button', { name: 'Go home', exact: true }).click();
});
