import { test, expect, type Page } from '@playwright/test';

const GOV_CONFIG: Record<string, unknown> = {
  repository: 'Agora/registry',
  environment: 'production',
  github_app_slug: null,
  development_registry: false,
};

const GOV_CONFIG_DEV: Record<string, unknown> = {
  repository: 'Agora/registry-dev',
  environment: 'sandbox',
  github_app_slug: 'agora-dev-bot',
  development_registry: true,
};

const GOV_EVENTS: Record<string, unknown>[] = [
  {
    event_id: 1,
    item_id: 'mod-001',
    event_type: 'status_change',
    status: 'resolved',
    detected_at: '2026-07-27T12:00:00Z',
    affected_reactions: 3,
    details_json: null,
  },
  {
    event_id: 2,
    item_id: 'mod-002',
    event_type: 'quarantine_flag',
    status: 'pending',
    detected_at: '2026-07-26T14:30:00Z',
    affected_reactions: 5,
    details_json: '{"reason":"Multiple user flags"}',
  },
];

const DIAGNOSTIC_CHECKS: Record<string, unknown>[] = [
  { id: 'governance_config', status: 'pass', message: 'Governance repository is properly configured.' },
  { id: 'under_review_items', status: 'warning', message: '2 items currently under review.' },
  { id: 'quarantine_flags', status: 'pass', message: 'No unresolved quarantine flags.' },
  { id: 'conflict_mapping', status: 'fail', message: 'Conflict mapping table is missing 3 entries.' },
];

const AUDIT_ENTRIES: Record<string, unknown>[] = [
  { id: 1, timestamp: '2026-07-27T08:00:00Z', action: 'triage_keep', details: 'Kept mod-001 after review' },
  { id: 2, timestamp: '2026-07-26T16:00:00Z', action: 'triage_archive', details: 'Archived mod-003 (abandoned)' },
];

const UNDER_REVIEW_ITEMS: Record<string, unknown>[] = [
  { id: 'mod-review-1', name: 'Review Mod 1', content_type: 'mod', icon_url: null, net_score: 12 },
  { id: 'mod-review-2', name: 'Review Mod 2', content_type: 'mod', icon_url: null, net_score: -3 },
];

const TRIAGE_POLLS: Record<string, Record<string, unknown>> = {
  'mod-review-1': { discussion_url: 'https://github.com/Agora/registry/issues/1', keep_votes: 5, remove_votes: 2 },
  'mod-review-2': { discussion_url: 'https://github.com/Agora/registry/issues/2', keep_votes: 1, remove_votes: 8 },
};

interface GovernanceMockOptions {
  config?: Record<string, unknown> | null;
  events?: Record<string, unknown>[];
  diagnosticChecks?: Record<string, unknown>[];
  auditEntries?: Record<string, unknown>[];
  resolutions?: Record<string, unknown>[];
  underReviewItems?: Record<string, unknown>[];
  triagePolls?: Record<string, Record<string, unknown>>;
  authenticated?: boolean;
  governanceSummary?: Record<string, unknown>;
}

async function installGovernanceMock(page: Page, opts: GovernanceMockOptions = {}) {
  const {
    config = GOV_CONFIG,
    events = GOV_EVENTS,
    diagnosticChecks = DIAGNOSTIC_CHECKS,
    auditEntries = AUDIT_ENTRIES,
    resolutions = AUDIT_ENTRIES,
    underReviewItems = UNDER_REVIEW_ITEMS,
    triagePolls = TRIAGE_POLLS,
    authenticated = true,
    governanceSummary = {
      item_id: 'mod-001',
      vote_issue_number: 42,
      vote_issue_url: 'https://github.com/Agora/registry/issues/42',
      raw_upvotes: 50,
      raw_downvotes: 5,
      counted_upvotes: 45,
      counted_downvotes: 3,
      quarantined_upvotes: 0,
      quarantined_downvotes: 0,
      conflicted_users: 2,
      status_reason: 'Active - no issues reported',
      compiled_at: '2026-07-27T12:00:00Z',
    },
  } = opts;

  await page.addInitScript(
    (params: Record<string, unknown>) => {
      const {
        config, events, diagnosticChecks, auditEntries, resolutions,
        underReviewItems, triagePolls, authenticated, governanceSummary,
      } = params as unknown as {
        config: Record<string, unknown> | null;
        events: Record<string, unknown>[];
        diagnosticChecks: Record<string, unknown>[];
        auditEntries: Record<string, unknown>[];
        resolutions: Record<string, unknown>[];
        underReviewItems: Record<string, unknown>[];
        triagePolls: Record<string, Record<string, unknown>>;
        authenticated: boolean;
        governanceSummary: Record<string, unknown>;
      };

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
          // Essential app bootstrapping
          if (command === 'get_setting') {
            const key = args.key as string;
            if (key === 'onboarding_complete') return Promise.resolve(true);
            if (key === 'ai_chat_enabled') return Promise.resolve(false);
            if (key === 'launch_mode') return Promise.resolve('delegation');
            if (key === 'last_home_visit') return Promise.resolve(null);
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

          // Governance
          if (command === 'get_governance_config') return Promise.resolve(config);
          if (command === 'get_governance_summary') {
            const itemId = args.itemId as string;
            return Promise.resolve({ ...governanceSummary, item_id: itemId });
          }
          if (command === 'list_governance_events') {
            const itemId = args.itemId as string | null;
            if (itemId) return Promise.resolve(events.filter((e) => e.item_id === itemId));
            return Promise.resolve(events);
          }
          if (command === 'run_governance_diagnostics') return Promise.resolve(diagnosticChecks);
          if (command === 'list_audit_log') return Promise.resolve(auditEntries);
          if (command === 'list_under_review_items') return Promise.resolve(underReviewItems);
          if (command === 'fetch_triage_poll') {
            const modId = args.modId as string;
            return Promise.resolve(triagePolls[modId] ?? null);
          }
          if (command === 'list_recent_resolutions') return Promise.resolve(resolutions);
          if (command === 'get_auth_status') return Promise.resolve(authenticated);

          // Stubs for other components mounted on the governance page
          if (command === 'list_categories') return Promise.resolve([]);
          if (command === 'list_manifest_loaders') return Promise.resolve([]);
          if (command === 'list_manifest_mc_versions') return Promise.resolve([]);
          if (command === 'list_instances') return Promise.resolve([]);
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
    {
      config, events, diagnosticChecks, auditEntries, resolutions,
      underReviewItems, triagePolls, authenticated, governanceSummary,
    },
  );
}

test.describe('Governance page compiled data', () => {

  test('shows governance configuration with repo and environment', async ({ page }) => {
    await installGovernanceMock(page);
    await page.goto('/');
    // Sidebar tabs are <button> elements, not <a>
    await page.getByRole('button', { name: 'Community Governance' }).click();

    await expect(page.getByText('Governance Configuration')).toBeVisible();
    await expect(page.getByText('Agora/registry', { exact: true })).toBeVisible();
    await expect(page.getByText('production', { exact: true })).toBeVisible();
    await expect(page.getByText('Production registry')).toBeVisible();
  });

  test('shows development registry mode', async ({ page }) => {
    await installGovernanceMock(page, { config: GOV_CONFIG_DEV });
    await page.goto('/');
    await page.getByRole('button', { name: 'Community Governance' }).click();

    await expect(page.getByText('sandbox', { exact: true })).toBeVisible();
    await expect(page.getByText('Development registry', { exact: true })).toBeVisible();
  });

});

test.describe('Governance page triage polls', () => {

  test('shows under review items with poll bars when authenticated', async ({ page }) => {
    await installGovernanceMock(page, { authenticated: true });
    await page.goto('/');
    await page.getByRole('button', { name: 'Community Governance' }).click();

    await expect(page.getByText('Review Mod 1')).toBeVisible();
    await expect(page.getByText('Review Mod 2')).toBeVisible();
    await expect(page.getByText('Score: 12')).toBeVisible();
    await expect(page.getByText('Score: -3')).toBeVisible();

    // Polls require explicit refresh per the governance page design
    await page.getByRole('button', { name: 'Refresh Polls' }).click();

    await expect(page.getByText(/Keep \d+%/).first()).toBeVisible();
    await expect(page.getByText(/Remove \d+%/).first()).toBeVisible();
    await expect(page.getByText('Cast Your Vote').first()).toBeVisible();
  });

  test('shows auth banner when not authenticated', async ({ page }) => {
    await installGovernanceMock(page, { authenticated: false });
    await page.goto('/');
    await page.getByRole('button', { name: 'Community Governance' }).click();

    await expect(page.getByText('Sign in with GitHub to see live triage poll results.')).toBeVisible();
    await expect(page.getByText('Cast Your Vote')).toHaveCount(0);
  });

  test('refresh polls button explicitly fetches updated data', async ({ page }) => {
    await installGovernanceMock(page, { authenticated: true });
    await page.goto('/');
    await page.getByRole('button', { name: 'Community Governance' }).click();

    const refreshBtn = page.getByRole('button', { name: 'Refresh Polls' });
    await expect(refreshBtn).toBeVisible();
    await refreshBtn.click();
    await expect(page.getByText(/Keep \d+%/).first()).toBeVisible();
  });

});

test.describe('Governance page events', () => {

  test('shows governance events with status and reactions', async ({ page }) => {
    await installGovernanceMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Community Governance' }).click();

    await expect(page.getByRole('heading', { name: 'Governance Events' })).toBeVisible();
    await expect(page.getByText('status_change').first()).toBeVisible();
    await expect(page.getByText('resolved')).toBeVisible();
    await expect(page.getByText('quarantine_flag')).toBeVisible();
    await expect(page.getByText('pending')).toBeVisible();
    await expect(page.getByText('Reactions: 5')).toBeVisible();
  });

});

test.describe('Governance page decisions and audit', () => {

  test('shows decisions and audit section', async ({ page }) => {
    await installGovernanceMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Community Governance' }).click();

    await expect(page.getByText('Decisions & Audit')).toBeVisible();
    await expect(page.getByText('triage_keep').first()).toBeVisible();
    await expect(page.getByText('triage_archive').first()).toBeVisible();
  });

});

test.describe('Governance page triage links and diagnostics', () => {

  test('shows triage links with governance repository', async ({ page }) => {
    await installGovernanceMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Community Governance' }).click();

    await expect(page.getByText('Triage & Governance Links')).toBeVisible();
    await expect(page.getByText('Agora/registry', { exact: true })).toBeVisible();
    await expect(page.getByText('Governance Issues')).toBeVisible();
    await expect(page.getByText('Registry Pull Requests')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Run Diagnostics' })).toBeVisible();
  });

  test('run diagnostics renders DiagnosticCheck[] items', async ({ page }) => {
    await installGovernanceMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Community Governance' }).click();

    await page.getByRole('button', { name: 'Run Diagnostics' }).click();

    await expect(page.getByText('Governance Diagnostics')).toBeVisible();
    await expect(page.getByText('governance_config')).toBeVisible();
    await expect(page.getByText('under_review_items')).toBeVisible();
    await expect(page.getByText('quarantine_flags')).toBeVisible();
    await expect(page.getByText('conflict_mapping')).toBeVisible();
    await expect(page.getByText('pass').first()).toBeVisible();
    await expect(page.getByText('warning')).toBeVisible();
    await expect(page.getByText('fail')).toBeVisible();
  });

  test('diagnostics shows check messages', async ({ page }) => {
    await installGovernanceMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Community Governance' }).click();

    await page.getByRole('button', { name: 'Run Diagnostics' }).click();
    await expect(page.getByText('Governance repository is properly configured.')).toBeVisible();
    await expect(page.getByText('2 items currently under review.')).toBeVisible();
  });

});

test.describe('Governance page vote mapping states', () => {

  test('shows latest event timestamp as compile time indicator', async ({ page }) => {
    await installGovernanceMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Community Governance' }).click();

    await expect(page.getByText('Latest event')).toBeVisible();
  });

});

test.describe('Governance page transparency log', () => {

  test('shows transparency log', async ({ page }) => {
    await installGovernanceMock(page);
    await page.goto('/');
    await page.getByRole('button', { name: 'Community Governance' }).click();

    await expect(page.getByText('Transparency Log')).toBeVisible();
    await expect(page.getByText('triage_keep').first()).toBeVisible();
    await expect(page.getByText('Kept mod-001 after review').first()).toBeVisible();
  });

});
