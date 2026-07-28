import { test, expect, type Page } from '@playwright/test';

const REGISTRY_ITEM: Record<string, unknown> = {
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

const CURATED_ANNOTATION: Record<string, unknown> = {
  id: 'test-mod',
  name: 'Test Mod',
  curator_note: 'A well-maintained mod.',
  net_score: 8,
  is_immune: false,
  base_categories: ['performance'],
};

const GOV_CONFIG: Record<string, unknown> = {
  repository: 'Agora/registry',
  environment: 'production',
  github_app_slug: null,
  development_registry: false,
};

const GOV_SUMMARY_NO_VOTE: Record<string, unknown> = {
  item_id: 'test-mod',
  vote_issue_number: null,
  vote_issue_url: null,
  raw_upvotes: 10,
  raw_downvotes: 2,
  counted_upvotes: 8,
  counted_downvotes: 1,
  quarantined_upvotes: 0,
  quarantined_downvotes: 0,
  conflicted_users: 0,
  status_reason: 'Active',
  compiled_at: '2026-07-27T12:00:00Z',
};

const GOV_SUMMARY_WITH_VOTE: Record<string, unknown> = {
  item_id: 'test-mod',
  vote_issue_number: 42,
  vote_issue_url: 'https://github.com/Agora/registry/issues/42',
  raw_upvotes: 10,
  raw_downvotes: 2,
  counted_upvotes: 8,
  counted_downvotes: 1,
  quarantined_upvotes: 2,
  quarantined_downvotes: 1,
  conflicted_users: 3,
  status_reason: 'Under review due to conflict flags',
  compiled_at: '2026-07-27T12:00:00Z',
};

interface ModDetailMockOptions {
  config?: Record<string, unknown> | null;
  governanceSummary?: Record<string, unknown>;
  showQuarantine?: boolean;
  registryItem?: Record<string, unknown>;
}

async function installModDetailMock(
  page: Page,
  opts: ModDetailMockOptions = {},
) {
  const {
    config = GOV_CONFIG,
    governanceSummary = GOV_SUMMARY_NO_VOTE,
    showQuarantine = false,
    registryItem = REGISTRY_ITEM,
  } = opts;

  const summary = showQuarantine
    ? { ...governanceSummary, quarantined_upvotes: 3, quarantined_downvotes: 1 }
    : governanceSummary;

  // Place the __agora state so the app starts on mod-detail
  const initDest = { type: 'mod-detail' as const, itemId: 'test-mod' };

  await page.addInitScript(
    (params: Record<string, unknown>) => {
      const {
        config, summary, registryItem, initDest,
      } = params as unknown as {
        config: Record<string, unknown> | null;
        summary: Record<string, unknown>;
        registryItem: Record<string, unknown>;
        initDest: { type: string; itemId: string };
      };

      // Seed the history state so useDestination restores mod-detail on boot
      window.history.replaceState({ __agora: initDest }, '');

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
          // App bootstrapping
          if (command === 'get_setting') {
            const key = args.key as string;
            if (key === 'onboarding_complete') return Promise.resolve(true);
            if (key === 'ai_chat_enabled') return Promise.resolve(false);
            if (key === 'launch_mode') return Promise.resolve('delegation');
            if (key === 'modrinth_enabled') return Promise.resolve(false);
            return Promise.resolve(null);
          }
          if (command === 'set_setting') return Promise.resolve(null);
          if (command === 'get_registry_status') return Promise.resolve({
            has_cached_db: true,
            cached_tag: 'test',
            cached_schema_version: 5,
            latest_tag: 'test',
            update_available: false,
            checked: true,
            message: 'Registry ready.',
          });
          if (command === 'check_registry_update') return Promise.resolve({
            has_cached_db: true,
            cached_tag: 'test',
            cached_schema_version: 5,
            latest_tag: 'test',
            update_available: false,
            checked: true,
            message: 'Registry ready.',
          });

          // Registry data
          if (command === 'get_registry_item') {
            const id = args.itemId as string;
            if (id === registryItem.id) return Promise.resolve(registryItem);
            return Promise.resolve(null);
          }
          if (command === 'get_curated_annotation') {
            return Promise.resolve({
              id: 'test-mod',
              name: 'Test Mod',
              curator_note: 'A well-maintained mod.',
              net_score: 8,
              is_immune: false,
              base_categories: ['performance'],
            });
          }
          if (command === 'get_governance_summary') {
            const itemId = args.itemId as string;
            return Promise.resolve({ ...summary, item_id: itemId });
          }
          if (command === 'get_governance_config') return Promise.resolve(config);
          if (command === 'list_mod_reviews') return Promise.resolve([
            {
              author: 'reviewer1',
              text: 'This mod works well with Fabric 1.21.',
              issue_number: 100,
              created_at: '2026-07-25T10:00:00Z',
              item_version: '1.0.0',
              minecraft_version: '1.21',
              loader: 'fabric',
            },
          ]);
          if (command === 'list_instances') return Promise.resolve([]);
          if (command === 'is_modrinth_enabled') return Promise.resolve(false);
          if (command === 'list_manifest_loaders') return Promise.resolve([]);
          if (command === 'list_manifest_mc_versions') return Promise.resolve([]);
          if (command === 'list_categories') return Promise.resolve([]);
          if (command === 'get_windows_accent_color') return Promise.resolve(null);
          if (command.startsWith('plugin:event|')) return Promise.resolve(1);

          return Promise.resolve(null);
        },
      };
      Object.assign(window as unknown as Record<string, unknown>, {
        __TAURI_INTERNALS__: internals,
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      });
    },
    { config, summary, registryItem, initDest },
  );
}

test.describe('ModDetail Governance / Agora tab', () => {

  test('has no Flag button in reviews', async ({ page }) => {
    await installModDetailMock(page);
    await page.goto('/');

    // The app initializes from the history state seeded above, so ModDetail
    // renders immediately. The Agora tab appears because get_curated_annotation
    // returns data.
    const agoraTab = page.getByRole('button', { name: 'Agora' });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    await expect(page.getByText('Flag', { exact: true })).toHaveCount(0);
    await expect(page.getByText(/flag limit|rate limit|flag review/i)).toHaveCount(0);
  });

  test('shows Vote on GitHub as disabled when no vote_issue_url', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_NO_VOTE });
    await page.goto('/');

    const agoraTab = page.getByRole('button', { name: 'Agora' });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    const voteControl = page.getByText('Vote on GitHub');
    await expect(voteControl).toBeVisible();
    // Not a link (no href) — disabled/empty state
    await expect(page.getByRole('link', { name: 'Vote on GitHub' })).toHaveCount(0);
  });

  test('shows Vote on GitHub as active link when vote_issue_url exists', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');

    const agoraTab = page.getByRole('button', { name: 'Agora' });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    const voteLink = page.getByRole('link', { name: 'Vote on GitHub' });
    await expect(voteLink).toBeVisible();
    await expect(voteLink).toHaveAttribute('href', 'https://github.com/Agora/registry/issues/42');
  });

  test('shows Write a technical review link when governance config exists', async ({ page }) => {
    await installModDetailMock(page);
    await page.goto('/');

    const agoraTab = page.getByRole('button', { name: 'Agora' });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    const reviewLink = page.getByRole('link', { name: 'Write a technical review' });
    await expect(reviewLink).toBeVisible();
    await expect(reviewLink).toHaveAttribute(
      'href',
      'https://github.com/Agora/registry/issues/new?template=review-form.yml',
    );
  });

  test('shows quarantine notice when quarantine votes exist', async ({ page }) => {
    await installModDetailMock(page, {
      governanceSummary: GOV_SUMMARY_WITH_VOTE,
      showQuarantine: true,
    });
    await page.goto('/');

    const agoraTab = page.getByRole('button', { name: 'Agora' });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    await expect(page.getByText('Quarantined', { exact: true })).toBeVisible();
    await expect(page.getByText(/voting activity currently under review/)).toBeVisible();
  });

  test('shows Conflicted Voters notice when conflicted_users > 0', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');

    const agoraTab = page.getByRole('button', { name: 'Agora' });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    await expect(page.getByText('Conflicted Voters', { exact: true })).toBeVisible();
    await expect(page.getByText('3 voters flagged conflicts on this item.')).toBeVisible();
  });

  test('displays governance summary compiled_at', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');

    const agoraTab = page.getByRole('button', { name: 'Agora' });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    await expect(page.getByText(/Compiled at/)).toBeVisible();
    // Verify the compiled timestamp contains expected date content
    const compiledText = await page.getByText(/Compiled at/).textContent();
    expect(compiledText).toContain('2026');
  });

});

test.describe('ModDetail Governance summary fields', () => {

  test('shows exact governance summary labels', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');

    const agoraTab = page.getByRole('button', { name: 'Agora' });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    await expect(page.getByText('Counted upvotes')).toBeVisible();
    await expect(page.getByText('Counted downvotes')).toBeVisible();
    await expect(page.getByText('Net score', { exact: true })).toBeVisible();
    await expect(page.getByText('Quarantined votes')).toBeVisible();
    await expect(page.getByText('Conflicted voters', { exact: true })).toBeVisible();
    await expect(page.getByText('Status reason')).toBeVisible();
    await expect(page.getByText('Under review due to conflict flags')).toBeVisible();
  });

});
