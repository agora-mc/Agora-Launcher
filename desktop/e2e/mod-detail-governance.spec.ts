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
  authenticated?: boolean;
  itemVoteState?: { vote: 'upvote' | 'downvote' | null; conflicted: boolean };
  voteError?: string | null;
  curatorWarning?: string | null;
  /** Holds set_item_vote open this long, so a vote stays in flight while the test presses keys. */
  voteDelayMs?: number;
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
    authenticated = true,
    itemVoteState = { vote: null, conflicted: false },
    voteError = null,
    curatorWarning = null,
    voteDelayMs = 0,
  } = opts;

  const summary = showQuarantine
    ? { ...governanceSummary, quarantined_upvotes: 3, quarantined_downvotes: 1 }
    : governanceSummary;

  // Place the __agora state so the app starts on mod-detail
  const initDest = { type: 'mod-detail' as const, itemId: 'test-mod' };

  await page.addInitScript(
    (params: Record<string, unknown>) => {
      const {
        config, summary, registryItem, initDest, authenticated, itemVoteState, voteError, curatorWarning, voteDelayMs,
      } = params as unknown as {
        config: Record<string, unknown> | null;
        summary: Record<string, unknown>;
        registryItem: Record<string, unknown>;
        initDest: { type: string; itemId: string };
        authenticated: boolean;
        itemVoteState: { vote: 'upvote' | 'downvote' | null; conflicted: boolean };
        voteError: string | null;
        curatorWarning: string | null;
        voteDelayMs: number;
      };

      // Seed the history state so useDestination restores mod-detail on boot
      window.history.replaceState({ __agora: initDest }, '');

      const callbacks = new Map<number, (...args: unknown[]) => void>();
      let callbackId = 0;
      let currentVote = itemVoteState;
      const voteCalls: Record<string, unknown>[] = [];

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
            if (key === 'launch_mode') return Promise.resolve('delegation');
            if (key === 'modrinth_enabled') return Promise.resolve(false);
            return Promise.resolve(null);
          }
          if (command === 'set_setting') return Promise.resolve(null);
          if (command === 'get_auth_status') return Promise.resolve(authenticated);
          if (command === 'get_github_profile') return Promise.resolve({ login: 'octocat', avatar_url: '' });
          if (command === 'get_registry_status') return Promise.resolve({
            has_cached_db: true,
            cached_tag: 'test',
            cached_schema_version: 5,
            latest_tag: 'test',
            update_available: false,
            checked: true,
            message: 'Catalog ready.',
          });
          if (command === 'check_registry_update') return Promise.resolve({
            has_cached_db: true,
            cached_tag: 'test',
            cached_schema_version: 5,
            latest_tag: 'test',
            update_available: false,
            checked: true,
            message: 'Catalog ready.',
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
              curator_warning: curatorWarning,
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
          if (command === 'get_item_vote') return Promise.resolve(currentVote);
          if (command === 'set_item_vote') {
            voteCalls.push({ ...args });
            if (voteError) return Promise.reject({ code: 'ERR_GOVERNANCE_VOTE', message: voteError });
            currentVote = {
              vote: (args.vote as 'upvote' | 'downvote' | null) ?? null,
              conflicted: false,
            };
            const landed = currentVote;
            return new Promise((resolve) => setTimeout(() => resolve(landed), voteDelayMs));
          }
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
        __voteCalls: voteCalls,
      });
    },
    { config, summary, registryItem, initDest, authenticated, itemVoteState, voteError, curatorWarning, voteDelayMs },
  );
}

test.describe('ModDetail Governance / Agora tab', () => {

  test('shows a curator warning on the page without opening the Agora tab', async ({ page }) => {
    const warning = 'Do not use XaeroPlus with this mod. The author says XaeroPlus is not affiliated with them.';
    await installModDetailMock(page, { curatorWarning: warning });
    await page.goto('/');

    const note = page.getByRole('note', { name: 'Curator warning' });
    await expect(note).toBeVisible();
    await expect(note).toContainText(warning);
  });

  test('shows no curator warning when the item has none', async ({ page }) => {
    await installModDetailMock(page);
    await page.goto('/');

    await expect(page.getByRole('note', { name: 'Curator warning' })).toHaveCount(0);
  });

  test('has no Flag button in reviews', async ({ page }) => {
    await installModDetailMock(page);
    await page.goto('/');

    // The app initializes from the history state seeded above, so ModDetail
    // renders immediately. The Agora tab appears because get_curated_annotation
    // returns data.
    const agoraTab = page.getByRole('button', { name: 'Agora', exact: true });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    await expect(page.getByText('Flag', { exact: true })).toHaveCount(0);
    await expect(page.getByText(/flag limit|rate limit|flag review/i)).toHaveCount(0);
  });

  test('shows Vote on GitHub as disabled when no vote_issue_url', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_NO_VOTE });
    await page.goto('/');

    const agoraTab = page.getByRole('button', { name: 'Agora', exact: true });
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

    const agoraTab = page.getByRole('button', { name: 'Agora', exact: true });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    const voteLink = page.getByRole('link', { name: 'Vote on GitHub' });
    await expect(voteLink).toBeVisible();
    await expect(voteLink).toHaveAttribute('href', 'https://github.com/Agora/registry/issues/42');
  });

  test('shows Write a technical review link when governance config exists', async ({ page }) => {
    await installModDetailMock(page);
    await page.goto('/');

    const agoraTab = page.getByRole('button', { name: 'Agora', exact: true });
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

    const agoraTab = page.getByRole('button', { name: 'Agora', exact: true });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    await expect(page.getByText('Quarantined', { exact: true })).toBeVisible();
    await expect(page.getByText(/voting activity currently under review/)).toBeVisible();
  });

  test('shows Conflicted Voters notice when conflicted_users > 0', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');

    const agoraTab = page.getByRole('button', { name: 'Agora', exact: true });
    await expect(agoraTab).toBeVisible();
    await agoraTab.click();

    await expect(page.getByText('Conflicted Voters', { exact: true })).toBeVisible();
    await expect(page.getByText('3 voters flagged conflicts on this item.')).toBeVisible();
  });

  test('displays governance summary compiled_at', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');

    const agoraTab = page.getByRole('button', { name: 'Agora', exact: true });
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

    const agoraTab = page.getByRole('button', { name: 'Agora', exact: true });
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

test.describe('ModDetail canonical voting controls', () => {
  // These tests are about the vote itself; the first-vote explainer has its own describe.
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem('agora-vote-explainer-acknowledged', '1'));
  });

  test('shows arrow controls with authoritative counts for mapped curated items', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');

    const upvote = page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' });
    const downvote = page.getByRole('button', { name: 'Downvote Test Mod. 2 downvotes.' });
    await expect(upvote).toBeEnabled();
    await expect(downvote).toBeEnabled();
    await expect(upvote.locator('svg')).toHaveCount(1);
    await expect(downvote.locator('svg')).toHaveCount(1);
  });

  test('keeps the arrow controls in their own community area, out of the install row', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');

    const community = page.getByTestId('mod-detail-community');
    await expect(community.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' })).toBeVisible();
    await expect(community.getByRole('button', { name: 'Downvote Test Mod. 2 downvotes.' })).toBeVisible();
    await expect(community.getByRole('button', { name: 'Install to Instance' })).toHaveCount(0);
  });

  test('records an upvote without changing the published count', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');

    const upvote = page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' });
    await upvote.click();

    await expect(upvote).toHaveAttribute('aria-pressed', 'true');
    await expect(upvote).toContainText('10');
    await expect(page.getByText(/Upvote recorded\. Scores update after the next catalog build/)).toBeVisible();
    const calls = await page.evaluate(() => (window as unknown as { __voteCalls: unknown[] }).__voteCalls);
    expect(calls).toEqual([{ itemId: 'test-mod', vote: 'upvote' }]);
  });

  test('keeps keyboard focus on the arrow after a vote lands', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE, voteDelayMs: 300 });
    await page.goto('/');

    const upvote = page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' });
    await upvote.focus();
    await page.keyboard.press('Enter');

    await expect(upvote).toHaveAttribute('aria-pressed', 'true');
    await expect(upvote).toBeFocused();
  });

  test('a held Enter, or a press during the vote, casts only one vote', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE, voteDelayMs: 1000 });
    await page.goto('/');

    const upvote = page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' });
    await upvote.focus();
    await page.keyboard.down('Enter');
    await page.keyboard.down('Enter');
    await page.keyboard.up('Enter');
    await page.keyboard.press('Enter');
    await expect(upvote).toHaveAttribute('aria-pressed', 'true');

    let calls = await page.evaluate(() => (window as unknown as { __voteCalls: unknown[] }).__voteCalls);
    expect(calls).toEqual([{ itemId: 'test-mod', vote: 'upvote' }]);

    // Once the vote has landed the control works again: a fresh press retracts it.
    await page.keyboard.press('Enter');
    await expect(upvote).toHaveAttribute('aria-pressed', 'false');
    calls = await page.evaluate(() => (window as unknown as { __voteCalls: unknown[] }).__voteCalls);
    expect(calls).toEqual([{ itemId: 'test-mod', vote: 'upvote' }, { itemId: 'test-mod', vote: null }]);
  });

  test('clicking the selected direction retracts the vote', async ({ page }) => {
    await installModDetailMock(page, {
      governanceSummary: GOV_SUMMARY_WITH_VOTE,
      itemVoteState: { vote: 'upvote', conflicted: false },
    });
    await page.goto('/');

    const upvote = page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' });
    await expect(upvote).toHaveAttribute('aria-pressed', 'true');
    await upvote.click();
    await expect(upvote).toHaveAttribute('aria-pressed', 'false');
    const calls = await page.evaluate(() => (window as unknown as { __voteCalls: unknown[] }).__voteCalls);
    expect(calls).toEqual([{ itemId: 'test-mod', vote: null }]);
  });

  test('switches vote direction and repairs conflicted reactions', async ({ page }) => {
    await installModDetailMock(page, {
      governanceSummary: GOV_SUMMARY_WITH_VOTE,
      itemVoteState: { vote: null, conflicted: true },
    });
    await page.goto('/');

    await expect(page.getByText(/Conflicting reactions detected/)).toBeVisible();
    await page.getByRole('button', { name: 'Downvote Test Mod. 2 downvotes.' }).click();
    const calls = await page.evaluate(() => (window as unknown as { __voteCalls: unknown[] }).__voteCalls);
    expect(calls).toEqual([{ itemId: 'test-mod', vote: 'downvote' }]);
  });

  test('disables in-app voting when GitHub is signed out', async ({ page }) => {
    await installModDetailMock(page, {
      governanceSummary: GOV_SUMMARY_WITH_VOTE,
      authenticated: false,
    });
    await page.goto('/');

    await expect(page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' })).toBeDisabled();
    await expect(page.getByText('Sign in with GitHub in Settings to vote.')).toBeVisible();
  });

  test('does not render vote buttons for immune or unmapped items', async ({ page }) => {
    await installModDetailMock(page, {
      governanceSummary: GOV_SUMMARY_WITH_VOTE,
      registryItem: { ...REGISTRY_ITEM, is_immune: true, immunity_reason: 'Protected.' },
    });
    await page.goto('/');
    await expect(page.getByRole('button', { name: /Upvote Test Mod/ })).toHaveCount(0);
  });

  test('rolls back selection and reports a failed vote', async ({ page }) => {
    await installModDetailMock(page, {
      governanceSummary: GOV_SUMMARY_WITH_VOTE,
      itemVoteState: { vote: 'upvote', conflicted: false },
      voteError: 'GitHub rejected the vote.',
    });
    await page.goto('/');

    const upvote = page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' });
    await page.getByRole('button', { name: 'Downvote Test Mod. 2 downvotes.' }).click();
    await expect(upvote).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByRole('alert')).toContainText('GitHub rejected the vote.');
  });
});

test.describe('ModDetail first-vote explainer', () => {
  const voteCalls = (page: import('@playwright/test').Page) =>
    page.evaluate(() => (window as unknown as { __voteCalls: unknown[] }).__voteCalls);

  test('the first vote asks first and sends nothing until confirmed', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');
    const upvote = page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' });
    await upvote.click();

    const dialog = page.getByRole('dialog');
    await expect(dialog.getByText('Votes are public')).toBeVisible();
    await expect(dialog.getByText('@octocat')).toBeVisible();
    expect(await voteCalls(page)).toEqual([]);

    await dialog.getByRole('button', { name: 'Vote publicly' }).click();
    await expect(upvote).toHaveAttribute('aria-pressed', 'true');
    expect(await voteCalls(page)).toEqual([{ itemId: 'test-mod', vote: 'upvote' }]);

    // Acknowledged: the next vote goes straight through.
    await page.getByRole('button', { name: 'Downvote Test Mod. 2 downvotes.' }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect.poll(async () => (await voteCalls(page)).length).toBe(2);
    expect(await page.evaluate(() => localStorage.getItem('agora-vote-explainer-acknowledged'))).toBe('1');
  });

  test('cancelling sends nothing and asks again next time', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');
    const upvote = page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' });
    await upvote.click();
    await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    expect(await voteCalls(page)).toEqual([]);
    await expect(upvote).toHaveAttribute('aria-pressed', 'false');
    expect(await page.evaluate(() => localStorage.getItem('agora-vote-explainer-acknowledged'))).toBeNull();

    await upvote.click();
    await expect(page.getByRole('dialog').getByText('Votes are public')).toBeVisible();
  });
});

test.describe('ModDetail vote arrows and keyboard use', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => localStorage.setItem('agora-vote-explainer-acknowledged', '1'));
  });

  /** Accessible names of the focused element, one per Tab press, until one of the targets is reached. */
  async function tabUntil(page: Page, targets: string[], maxPresses: number): Promise<string[]> {
    const seen: string[] = [];
    for (let i = 0; i < maxPresses; i += 1) {
      await page.keyboard.press('Tab');
      const name = await page.evaluate(() => {
        const el = document.activeElement as HTMLElement | null;
        return el?.getAttribute('aria-label') ?? el?.textContent?.trim() ?? el?.tagName ?? '';
      });
      seen.push(name);
      if (targets.some((target) => name === target)) break;
    }
    return seen;
  }

  test('Tab reaches Install to Instance before the vote arrows', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');
    await expect(page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' })).toBeEnabled();

    const seen = await tabUntil(page, ['Install to Instance', 'Upvote Test Mod. 10 upvotes.'], 60);
    expect(seen[seen.length - 1]).toBe('Install to Instance');
    expect(seen).not.toContain('Upvote Test Mod. 10 upvotes.');
  });

  test('holding Enter on an arrow records one vote, not one per key repeat', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');

    const upvote = page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' });
    await upvote.focus();
    await page.keyboard.down('Enter');
    // Each repeat arrives after the previous vote has settled. Today the arrow is
    // disabled while its vote is in flight, which drops focus, so the repeats find
    // nothing to press. This guards that: it fails if focus is kept on the arrow
    // without something else stopping the repeats.
    for (let i = 0; i < 3; i += 1) {
      await page.waitForTimeout(150);
      await page.keyboard.down('Enter');
    }
    await page.keyboard.up('Enter');
    await page.waitForTimeout(150);

    const calls = await page.evaluate(() => (window as unknown as { __voteCalls: unknown[] }).__voteCalls);
    expect(calls).toEqual([{ itemId: 'test-mod', vote: 'upvote' }]);
    await expect(upvote).toHaveAttribute('aria-pressed', 'true');
  });

  test('a single Enter still votes', async ({ page }) => {
    await installModDetailMock(page, { governanceSummary: GOV_SUMMARY_WITH_VOTE });
    await page.goto('/');

    const upvote = page.getByRole('button', { name: 'Upvote Test Mod. 10 upvotes.' });
    await upvote.focus();
    await page.keyboard.press('Enter');
    await expect(upvote).toHaveAttribute('aria-pressed', 'true');
  });
});
