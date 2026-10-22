import { test, expect, type Page } from '@playwright/test';
import { TOUR_STEPS } from '../src/features/tour/tourModel';

// ---------------------------------------------------------------------------
// One Tauri mock for the walkthrough and onboarding checks in this file. Any
// command it does not name returns null, so every screen these tests visit
// must be covered here; an unmocked command crashes the page under test.
// ---------------------------------------------------------------------------

interface MockOptions {
  onboardingComplete: boolean;
  /** Resume onboarding at this step (a `Step` id). */
  onboardingStep?: string;
  /** Whether a GitHub token is already stored. */
  githubStored?: boolean;
}

async function installMock(page: Page, options: MockOptions) {
  await page.addInitScript(({ options }) => {
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    let callbackId = 0;
    let createdInstance: Record<string, unknown> | null = null;

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
          if (key === 'onboarding_complete') return Promise.resolve(options.onboardingComplete);
          if (key === 'onboarding_step') return Promise.resolve(options.onboardingStep ?? null);
          if (key === 'launch_mode') return Promise.resolve('delegation');
          if (key === 'last_home_visit') return Promise.resolve(null);
          return Promise.resolve(false);
        }
        if (command === 'set_setting') return Promise.resolve(null);
        if (command === 'get_auth_status') return Promise.resolve(options.githubStored === true);
        if (command === 'get_github_profile') {
          return Promise.resolve(options.githubStored ? { login: 'octocat', avatar_url: '' } : null);
        }
        if (command === 'get_windows_accent_color') return Promise.resolve(null);
        if (command === 'get_registry_status') {
          return Promise.resolve({
            has_cached_db: true,
            cached_tag: 'test',
            cached_schema_version: 5,
            latest_tag: 'test',
            update_available: false,
            checked: true,
            message: 'Catalog ready.',
          });
        }
        if (command === 'check_registry_update') return Promise.resolve(null);
        if (command === 'list_categories') return Promise.resolve([]);
        if (command === 'browse_search') return Promise.resolve({ items: [], total: 0, page: 0, hasMore: false });
        if (command === 'for_you_items') return Promise.resolve([]);
        if (command === 'get_lkg_marker') return Promise.resolve(null);
        if (command === 'list_manifest_loaders') return Promise.resolve(['fabric']);
        if (command === 'list_manifest_mc_versions') return Promise.resolve(['1.21']);
        if (command === 'list_loader_versions') {
          return Promise.resolve([
            { loader: 'fabric', mc_version: '1.21', loader_version: '0.16.9', file_type: 'stable' },
          ]);
        }
        if (command === 'list_instances') return Promise.resolve(createdInstance ? [createdInstance] : []);
        if (command === 'create_instance') {
          const req = args.request as Record<string, unknown>;
          createdInstance = {
            instance_id: req.instance_id,
            name: req.name,
            minecraft_version: req.minecraft_version,
            loader: req.loader,
            loader_version: req.loader_version,
            is_locked: false,
            last_launched_at: null,
          };
          return Promise.resolve(createdInstance);
        }
        if (command === 'check_instance_crash') return Promise.resolve(null);
        if (command === 'check_instance_updates') return Promise.resolve([]);
        if (command === 'list_instance_templates') return Promise.resolve([]);
        if (command === 'query_launch_state') return Promise.resolve([]);
        if (command === 'plugin:event|listen') return Promise.resolve(1);
        if (command === 'plugin:event|unlisten') return Promise.resolve(null);
        if (command.startsWith('plugin:event|')) return Promise.resolve(1);
        return Promise.resolve(null);
      },
    };
    Object.assign(window as unknown as Record<string, unknown>, {
      __TAURI_INTERNALS__: internals,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
    });
  }, { options });
}

/** Seed a running walkthrough at the step with this id. */
async function resumeTourAt(page: Page, stepId: string) {
  const index = TOUR_STEPS.findIndex((step) => step.id === stepId);
  expect(index).toBeGreaterThan(-1);
  await page.addInitScript((i) => {
    localStorage.setItem('agora-tour', JSON.stringify({ version: 1, status: 'running', index: i, completed: false }));
  }, index);
}

// ---------------------------------------------------------------------------
// Walkthrough: creating an instance
// ---------------------------------------------------------------------------

test.describe('walkthrough: create an instance', () => {
  test('typing a name and pressing Create move the tour on, and it does not ask to reopen the dialog', async ({ page }) => {
    await resumeTourAt(page, 'create-instance-button');
    await installMock(page, { onboardingComplete: true });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();
    await page.getByRole('button', { name: 'Create Instance' }).click();

    const tour = page.getByRole('dialog', { name: 'Guided tour' });
    await expect(tour.getByText('Name it and leave the rest')).toBeVisible();

    // Typing in the name field moves the tour to the Create step by itself.
    await page.getByPlaceholder('Optimized Survival').fill('My Pack');
    await expect(tour.getByText('Create it')).toBeVisible();

    // Pressing Create finishes the step; the instance is made and the tour
    // moves on to Browse.
    await page.getByRole('button', { name: 'Create', exact: true }).click();
    await expect(tour.getByText('Now it needs mods')).toBeVisible();
    await expect(page.getByText('Open “+ Create Instance” again to continue.')).toHaveCount(0);
  });

  test('Skip on the Create Instance step goes straight to Browse, not to the dialog steps', async ({ page }) => {
    await resumeTourAt(page, 'create-instance-button');
    await installMock(page, { onboardingComplete: true });
    await page.goto('/');
    await page.getByRole('button', { name: 'My Instances' }).click();

    const tour = page.getByRole('dialog', { name: 'Guided tour' });
    await expect(tour.getByText('Create your first instance')).toBeVisible();
    await tour.getByRole('button', { name: 'Skip step' }).click();

    await expect(tour.getByText('Now it needs mods')).toBeVisible();
    await expect(page.getByText('Open “+ Create Instance” again to continue.')).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// Onboarding: GitHub step and appearance presets
// ---------------------------------------------------------------------------

test.describe('onboarding', () => {
  test('a stored GitHub token is shown as signed in, with Continue and no sign-in offer', async ({ page }) => {
    await installMock(page, { onboardingComplete: false, onboardingStep: 'github', githubStored: true });
    await page.goto('/');

    await expect(page.getByText('Signed in as', { exact: false })).toBeVisible();
    await expect(page.getByText('@octocat')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Sign in with GitHub' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Continue', exact: true })).toBeVisible();
  });

  test('without a stored token the GitHub step still offers sign-in', async ({ page }) => {
    await installMock(page, { onboardingComplete: false, onboardingStep: 'github', githubStored: false });
    await page.goto('/');

    await expect(page.getByRole('button', { name: 'Sign in with GitHub' })).toBeVisible();
    await expect(page.getByText(/Signed in as/)).toHaveCount(0);
  });

  test('the default appearance preset shows as selected on a fresh install', async ({ page }) => {
    await installMock(page, { onboardingComplete: false, onboardingStep: 'appearance' });
    await page.goto('/');

    await expect(page.getByRole('button', { name: /Civic Gold/ })).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByRole('button', { name: /Agora Blue/ })).toHaveAttribute('aria-pressed', 'false');

    await page.getByRole('button', { name: /Agora Blue/ }).click();
    await expect(page.getByRole('button', { name: /Agora Blue/ })).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByRole('button', { name: /Civic Gold/ })).toHaveAttribute('aria-pressed', 'false');
  });
});
