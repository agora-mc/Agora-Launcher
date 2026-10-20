import type { Page } from '@playwright/test';
import { expect } from '@playwright/test';

/**
 * A mock of Tauri's `__TAURI_INTERNALS__` for specs that exercise the multi-game shell.
 *
 * Unlike the older specs, which answer any unknown command with `null`, this one answers from a
 * per-test table and treats every other command as a bug in the test: it records the name and
 * rejects the call, and `expectNoUnmockedCommands` fails the test when anything was recorded.
 * A `null` reply therefore means "the command answered with nothing", never "I forgot it".
 *
 * Settings are the one stateful part. `get_setting` and `set_setting` read and write
 * `localStorage`, so a value written before a reload is still there after it, the way the real
 * settings store behaves across restarts.
 *
 * Tauri's own `plugin:*` protocol calls (event listeners and the like) are not application
 * commands and are answered without being recorded.
 */

/** Replies by command name. Values are plain data: they cross into the page as JSON. */
export interface GamesMockTable {
  replies: Record<string, unknown>;
  /** Settings the app starts with. Values written later by `set_setting` take precedence. */
  settings?: Record<string, unknown>;
}

const SETTINGS_STORE_KEY = '__agora_games_mock_settings';
const UNMOCKED_LOG_KEY = '__agora_games_mock_unmocked';

/** Installs the mock before any page script runs. Call it before `page.goto`. */
export async function installGamesMock(page: Page, table: GamesMockTable): Promise<void> {
  await page.addInitScript(
    ({ table, settingsKey, unmockedKey }) => {
      const replies = table.replies;
      const seeded: Record<string, unknown> = { onboarding_complete: true, ...table.settings };
      const stored = JSON.parse(localStorage.getItem(settingsKey) ?? '{}') as Record<string, unknown>;
      const settings: Record<string, unknown> = { ...seeded, ...stored };
      const persistSettings = () => localStorage.setItem(settingsKey, JSON.stringify(settings));
      persistSettings();

      const recordUnmocked = (command: string) => {
        const list = JSON.parse(sessionStorage.getItem(unmockedKey) ?? '[]') as string[];
        list.push(command);
        sessionStorage.setItem(unmockedKey, JSON.stringify(list));
      };

      Object.assign(window as unknown as Record<string, unknown>, {
        __TAURI_INTERNALS__: {
          transformCallback() { return 1; },
          unregisterCallback() {},
          invoke(command: string, args: Record<string, unknown> = {}) {
            if (command.startsWith('plugin:')) return Promise.resolve(1);
            if (command === 'get_setting') {
              const key = String(args.key);
              return Promise.resolve(Object.prototype.hasOwnProperty.call(settings, key) ? settings[key] : null);
            }
            if (command === 'set_setting') {
              settings[String(args.key)] = args.value;
              persistSettings();
              return Promise.resolve(null);
            }
            if (Object.prototype.hasOwnProperty.call(replies, command)) {
              return Promise.resolve(replies[command]);
            }
            recordUnmocked(command);
            return Promise.reject(new Error(`Unmocked Tauri command in a games test: ${command}`));
          },
        },
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      });
    },
    { table, settingsKey: SETTINGS_STORE_KEY, unmockedKey: UNMOCKED_LOG_KEY },
  );
}

/** Commands that were called without a table entry. Survives reloads within the test. */
export async function unmockedCommands(page: Page): Promise<string[]> {
  return page.evaluate((key) => JSON.parse(sessionStorage.getItem(key) ?? '[]') as string[], UNMOCKED_LOG_KEY);
}

/** The stored value of a setting, as the app last wrote it (or the table's seed if it never did). */
export async function storedSetting(page: Page, key: string): Promise<unknown> {
  return page.evaluate(
    ({ storeKey, key }) => (JSON.parse(localStorage.getItem(storeKey) ?? '{}') as Record<string, unknown>)[key] ?? null,
    { storeKey: SETTINGS_STORE_KEY, key },
  );
}

/** Fails the test if the app called a command the table does not answer. */
export async function expectNoUnmockedCommands(page: Page): Promise<void> {
  expect(await unmockedCommands(page), 'commands the games table does not answer').toEqual([]);
}
