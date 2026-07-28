import { useState, useEffect, useCallback } from 'react';
import {
  listAuditLog,
  formatError,
  AuditLogEntry,
  listUnderReviewItems,
  UnderReviewItem,
  fetchTriagePoll,
  TriagePoll,
  listRecentResolutions,
  getAuthStatus,
  isAuthExpired,
  getGovernanceConfig,
  listGovernanceEvents,
  runGovernanceDiagnostics,
  type GovernanceConfig,
  type GovernanceEvent,
  type DiagnosticCheck,
} from '../lib/tauri';

export function Governance() {
  const [config, setConfig] = useState<GovernanceConfig | null>(null);
  const [configLoading, setConfigLoading] = useState(true);

  const [logEntries, setLogEntries] = useState<AuditLogEntry[]>([]);
  const [logLoading, setLogLoading] = useState(true);
  const [logError, setLogError] = useState<string | null>(null);

  const [events, setEvents] = useState<GovernanceEvent[]>([]);
  const [eventsLoading, setEventsLoading] = useState(false);
  const [eventsError, setEventsError] = useState<string | null>(null);

  const [underReviewItems, setUnderReviewItems] = useState<UnderReviewItem[]>([]);
  const [polls, setPolls] = useState<Record<string, TriagePoll | null>>({});
  const [pollsLoading, setPollsLoading] = useState(true);
  const [pollsError, setPollsError] = useState<string | null>(null);
  const [pollsRefreshing, setPollsRefreshing] = useState(false);
  const [authenticated, setAuthenticated] = useState<boolean | null>(null);

  const [resolutions, setResolutions] = useState<AuditLogEntry[]>([]);
  const [resolutionsLoading, setResolutionsLoading] = useState(true);
  const [resolutionsError, setResolutionsError] = useState<string | null>(null);

  const [diagnostics, setDiagnostics] = useState<DiagnosticCheck[] | null>(null);
  const [diagnosticsLoading, setDiagnosticsLoading] = useState(false);
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);
  const [showDiagnostics, setShowDiagnostics] = useState(false);

  useEffect(() => {
    let cancelled = false;

    getGovernanceConfig()
      .then((c) => {
        if (!cancelled) setConfig(c);
      })
      .catch(() => {})
      .finally(() => {
        if (!cancelled) setConfigLoading(false);
      });

    listAuditLog(200)
      .then((data) => {
        if (!cancelled) setLogEntries(data);
      })
      .catch((e) => {
        if (!cancelled) setLogError(formatError(e));
      })
      .finally(() => {
        if (!cancelled) setLogLoading(false);
      });

    listGovernanceEvents(null)
      .then((data) => {
        if (!cancelled) setEvents(data);
      })
      .catch((e) => {
        if (!cancelled) setEventsError(formatError(e));
      })
      .finally(() => {
        if (!cancelled) setEventsLoading(false);
      });

    getAuthStatus()
      .then((auth) => {
        if (!cancelled) setAuthenticated(auth);
      })
      .catch(() => {
        if (!cancelled) setAuthenticated(false);
      });

    listUnderReviewItems()
      .then((items) => {
        if (!cancelled) setUnderReviewItems(items);
      })
      .catch((e) => {
        if (!cancelled) setPollsError(formatError(e));
      })
      .finally(() => {
        if (!cancelled) setPollsLoading(false);
      });

    listRecentResolutions(50)
      .then((data) => {
        if (!cancelled) setResolutions(data);
      })
      .catch((e) => {
        if (!cancelled) setResolutionsError(formatError(e));
      })
      .finally(() => {
        if (!cancelled) setResolutionsLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  const refreshPolls = useCallback(async () => {
    if (!authenticated || underReviewItems.length === 0) return;
    setPollsRefreshing(true);
    setPollsError(null);
    const results: Record<string, TriagePoll | null> = {};
    const errors: string[] = [];
    let sawAuthExpired = false;

    await Promise.all(
      underReviewItems.map(async (item) => {
        try {
          results[item.id] = await fetchTriagePoll(item.id);
        } catch (e) {
          if (isAuthExpired(e)) sawAuthExpired = true;
          errors.push(formatError(e));
          results[item.id] = null;
        }
      }),
    );

    if (sawAuthExpired) setAuthenticated(false);
    setPolls(results);
    if (errors.length > 0) setPollsError(errors.join('; '));
    setPollsRefreshing(false);
  }, [authenticated, underReviewItems]);

  const handleRunDiagnostics = useCallback(async () => {
    setDiagnosticsLoading(true);
    setDiagnosticsError(null);
    setDiagnostics(null);
    setShowDiagnostics(true);
    try {
      const result = await runGovernanceDiagnostics();
      setDiagnostics(result);
    } catch (e) {
      setDiagnosticsError(formatError(e));
    } finally {
      setDiagnosticsLoading(false);
    }
  }, []);

  const actionBadgeColor = (action: string): string => {
    switch (action) {
      case 'triage_archive':
        return 'bg-destructive/20 text-destructive';
      case 'triage_keep':
        return 'bg-green-200 dark:bg-green-900 text-green-800 dark:text-green-200';
      case 'organic_under_review':
        return 'bg-orange-200 dark:bg-orange-900 text-orange-800 dark:text-orange-200';
      case 'raid_breaker_offenders':
        return 'bg-destructive/20 text-destructive';
      default:
        return 'bg-muted';
    }
  };

  const openDiscussion = (url: string | null) => {
    if (url && url.startsWith('https://')) {
      window.open(url, '_blank');
    }
  };

  return (
    <div className="space-y-6">
      <section>
        <h2 className="text-2xl font-bold mb-2">Community Governance</h2>
        <p className="text-muted-foreground">
          Configured repository, triage polls, governance events, and audits.
        </p>
      </section>

      {/* Compiled data: configured repo / environment / compile time */}
      {!configLoading && config && (
        <section className="rounded-xl border border-border bg-card p-6">
          <h3 className="text-lg font-semibold mb-1">Governance Configuration</h3>
          <p className="text-xs text-muted-foreground mb-3">
            Repository, environment, registry mode, and data freshness detected at compile time.
          </p>
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 text-sm">
            <div>
              <span className="text-muted-foreground text-xs">Repository</span>
              <p className="font-medium break-all">{config.repository}</p>
            </div>
            <div>
              <span className="text-muted-foreground text-xs">Environment</span>
              <p className="font-medium capitalize">{config.environment}</p>
            </div>
            <div>
              <span className="text-muted-foreground text-xs">Mode</span>
              <p className="font-medium">
                {config.development_registry ? 'Development registry' : 'Production registry'}
              </p>
            </div>
            <div>
              <span className="text-muted-foreground text-xs">Latest event</span>
              <p className="font-medium text-xs">
                {events.length > 0
                  ? new Date(events[0].detected_at).toLocaleString()
                  : 'No events recorded'}
              </p>
            </div>
          </div>
        </section>
      )}

      {/* Auth banner */}
      {authenticated === false && (
        <div className="rounded-xl p-4 border border-dashed border-border bg-muted text-center">
          <p className="text-muted-foreground">
            Sign in with GitHub to see live triage poll results.
          </p>
        </div>
      )}

      {/* Active Triage Polls */}
      <section className="rounded-xl border border-border bg-card p-6">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-semibold">Active Triage Polls</h3>
          {authenticated && underReviewItems.length > 0 && (
            <button
              onClick={refreshPolls}
              disabled={pollsRefreshing}
              className="rounded-lg border border-border px-3 py-1.5 text-xs font-medium hover:bg-accent disabled:opacity-50 flex items-center gap-1.5"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                className={pollsRefreshing ? 'animate-spin' : ''}
              >
                <path d="M21 2v6h-6" />
                <path d="M3 12a9 9 0 0 1 15-6.7L21 8" />
                <path d="M3 22v-6h6" />
                <path d="M21 12a9 9 0 0 1-15 6.7L3 16" />
              </svg>
              {pollsRefreshing ? 'Refreshing...' : 'Refresh Polls'}
            </button>
          )}
        </div>

        {pollsError && (
          <div className="mb-4 p-4 rounded-lg bg-destructive/10 border border-destructive text-destructive">
            {pollsError}
          </div>
        )}

        {pollsLoading && (
          <p className="text-muted-foreground">Loading triage polls...</p>
        )}

        {!pollsLoading && !pollsError && underReviewItems.length === 0 && (
          <p className="text-muted-foreground">No items under review.</p>
        )}

        {!pollsLoading && !pollsError && underReviewItems.length > 0 && (
          <div className="space-y-3">
            {underReviewItems.map((item) => {
              const poll = polls[item.id] ?? null;
              const totalVotes = (poll?.keep_votes ?? 0) + (poll?.remove_votes ?? 0);
              const keepPct =
                totalVotes > 0
                  ? Math.round(((poll?.keep_votes ?? 0) / totalVotes) * 100)
                  : 0;
              const removePct = 100 - keepPct;
              const canViewPoll = authenticated && poll !== null;

              return (
                <div
                  key={item.id}
                  className="p-4 rounded-lg bg-muted border border-border"
                >
                  <div className="flex items-start gap-3">
                    {item.icon_url && (
                      <img
                        src={item.icon_url}
                        alt=""
                        className="w-8 h-8 rounded flex-shrink-0"
                      />
                    )}
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2 flex-wrap">
                        <span className="font-medium truncate">{item.name}</span>
                        <span className="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground capitalize">
                          {item.content_type}
                        </span>
                        <span className="text-xs px-2 py-0.5 rounded-full bg-blue-100 dark:bg-blue-900 text-blue-800 dark:text-blue-200">
                          Score: {item.net_score}
                        </span>
                      </div>

                      {canViewPoll && totalVotes > 0 && (
                        <div className="mt-3">
                          <div className="flex rounded-full overflow-hidden h-3 bg-muted">
                            <div
                              className="bg-green-500"
                              style={{ width: `${keepPct}%` }}
                            />
                            <div
                              className="bg-red-500"
                              style={{ width: `${removePct}%` }}
                            />
                          </div>
                          <div className="flex justify-between text-xs text-muted-foreground mt-1">
                            <span>Keep {keepPct}%</span>
                            <span>Remove {removePct}%</span>
                          </div>
                        </div>
                      )}

                      {canViewPoll && totalVotes === 0 && (
                        <p className="mt-2 text-sm text-muted-foreground">
                          No votes yet.
                        </p>
                      )}

                      {!authenticated && (
                        <p className="mt-2 text-sm text-muted-foreground">
                          Sign in to view live results
                        </p>
                      )}

                      {poll && poll.discussion_url ? (
                        <button
                          onClick={() => openDiscussion(poll.discussion_url)}
                          className="mt-2 text-sm text-blue-600 dark:text-blue-400 hover:underline"
                        >
                          Cast Your Vote
                        </button>
                      ) : (
                        poll && (
                          <p className="mt-2 text-sm text-muted-foreground">
                            Poll not available
                          </p>
                        )
                      )}
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>

      {/* Governance Events */}
      {!eventsLoading && events.length > 0 && (
        <section className="rounded-xl border border-border bg-card p-6">
          <h3 className="text-lg font-semibold mb-4">Governance Events</h3>
          <div className="max-h-80 overflow-y-auto space-y-3 pr-2">
            {events.map((evt) => (
              <div
                key={evt.event_id}
                className="p-3 rounded-lg bg-muted border border-border"
              >
                <div className="flex items-center gap-2 mb-1">
                  <time
                    dateTime={evt.detected_at}
                    className="text-sm text-muted-foreground font-mono"
                  >
                    {evt.detected_at}
                  </time>
                  <span className="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground capitalize">
                    {evt.event_type}
                  </span>
                  <span className="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground">
                    {evt.status}
                  </span>
                </div>
                {evt.item_id && (
                  <p className="text-xs text-muted-foreground">
                    Item: <code className="text-xs">{evt.item_id}</code> &middot;
                    Reactions: {evt.affected_reactions}
                  </p>
                )}
                {evt.details_json && (
                  <p className="text-xs text-muted-foreground mt-1">{evt.details_json}</p>
                )}
              </div>
            ))}
          </div>
        </section>
      )}

      {eventsError && (
        <div className="mb-4 p-4 rounded-lg bg-destructive/10 border border-destructive text-destructive">
          {eventsError}
        </div>
      )}

      {/* Decisions / Audit */}
      <section className="rounded-xl border border-border bg-card p-6">
        <h3 className="text-lg font-semibold mb-4">Decisions &amp; Audit</h3>

        {resolutionsError && (
          <div className="mb-4 p-4 rounded-lg bg-destructive/10 border border-destructive text-destructive">
            {resolutionsError}
          </div>
        )}

        {resolutionsLoading && (
          <p className="text-muted-foreground">Loading resolutions...</p>
        )}

        {!resolutionsLoading && !resolutionsError && resolutions.length === 0 && (
          <p className="text-muted-foreground">No triage resolutions yet.</p>
        )}

        {!resolutionsLoading && !resolutionsError && resolutions.length > 0 && (
          <div className="max-h-96 overflow-y-auto space-y-3 pr-2">
            {resolutions.map((entry) => (
              <div
                key={entry.id}
                className="p-3 rounded-lg bg-muted border border-border"
              >
                <div className="flex items-center gap-2 mb-1">
                  <time
                    dateTime={entry.timestamp}
                    className="text-sm text-muted-foreground font-mono"
                  >
                    {entry.timestamp}
                  </time>
                  <span
                    className={`text-xs px-2 py-0.5 rounded-full capitalize ${actionBadgeColor(
                      entry.action,
                    )}`}
                  >
                    {entry.action}
                  </span>
                </div>
                {entry.details && (
                  <p className="text-sm text-muted-foreground">{entry.details}</p>
                )}
              </div>
            ))}
          </div>
        )}
      </section>

      {/* Triage links when config present */}
      {config && (
        <section className="rounded-xl border border-border bg-card p-6">
          <h3 className="text-lg font-semibold mb-4">Triage &amp; Governance Links</h3>
          <p className="text-xs text-muted-foreground mb-3">
            Repository: {config.repository}
          </p>
          <div className="flex flex-wrap gap-3">
            <a
              href={`https://github.com/${config.repository}/issues`}
              target="_blank"
              rel="noopener noreferrer"
              className="rounded-lg border border-border px-4 py-2 text-sm font-medium hover:bg-accent"
            >
              Governance Issues
            </a>
            <a
              href={`https://github.com/${config.repository}/pulls`}
              target="_blank"
              rel="noopener noreferrer"
              className="rounded-lg border border-border px-4 py-2 text-sm font-medium hover:bg-accent"
            >
              Registry Pull Requests
            </a>
            <button
              onClick={handleRunDiagnostics}
              disabled={diagnosticsLoading}
              className="rounded-lg border border-border px-4 py-2 text-sm font-medium hover:bg-accent disabled:opacity-50"
            >
              {diagnosticsLoading ? 'Running diagnostics...' : 'Run Diagnostics'}
            </button>
          </div>
        </section>
      )}

      {/* Diagnostics UI */}
      {showDiagnostics && (
        <section className="rounded-xl border border-border bg-card p-6">
          <div className="flex items-center justify-between mb-4">
            <h3 className="text-lg font-semibold">Governance Diagnostics</h3>
            <button
              onClick={() => setShowDiagnostics(false)}
              className="text-xs text-muted-foreground hover:text-foreground"
            >
              Close
            </button>
          </div>

          {diagnosticsLoading && (
            <p className="text-muted-foreground">Running diagnostics...</p>
          )}

          {diagnosticsError && (
            <div className="p-4 rounded-lg bg-destructive/10 border border-destructive text-destructive">
              {diagnosticsError}
            </div>
          )}

          {diagnostics && !diagnosticsError && (
            <div className="space-y-2">
              {diagnostics.map((check) => (
                <div
                  key={check.id}
                  className={`rounded-lg border px-3 py-2 text-sm ${
                    check.status === 'pass'
                      ? 'border-green-500/50 bg-green-50/50 dark:bg-green-900/10'
                      : check.status === 'warning'
                        ? 'border-amber-500/50 bg-amber-50/50 dark:bg-amber-900/10'
                        : 'border-red-500/50 bg-red-50/50 dark:bg-red-900/10'
                  }`}
                >
                  <div className="flex items-center gap-2">
                    <span
                      className={`text-xs font-semibold uppercase ${
                        check.status === 'pass'
                          ? 'text-green-600 dark:text-green-400'
                          : check.status === 'warning'
                            ? 'text-amber-600 dark:text-amber-400'
                            : 'text-red-600 dark:text-red-400'
                      }`}
                    >
                      {check.status}
                    </span>
                    <span className="text-xs font-medium text-muted-foreground">
                      {check.id}
                    </span>
                  </div>
                  <p className="text-xs text-foreground mt-0.5">{check.message}</p>
                </div>
              ))}
            </div>
          )}
        </section>
      )}

      {/* Transparency Log */}
      <section className="rounded-xl border border-border bg-card p-6">
        <h3 className="text-lg font-semibold mb-4">Transparency Log</h3>

        {logError && (
          <div className="mb-4 p-4 rounded-lg bg-destructive/10 border border-destructive text-destructive">
            {logError}
          </div>
        )}

        {logLoading && (
          <p className="text-muted-foreground">Loading transparency log...</p>
        )}

        {!logLoading && !logError && logEntries.length === 0 && (
          <p className="text-muted-foreground">No governance actions recorded yet.</p>
        )}

        {!logLoading && !logError && logEntries.length > 0 && (
          <div className="max-h-96 overflow-y-auto space-y-3 pr-2">
            {logEntries.map((entry) => (
              <div
                key={entry.id}
                className="p-3 rounded-lg bg-muted border border-border"
              >
                <div className="flex items-center gap-2 mb-1">
                  <time
                    dateTime={entry.timestamp}
                    className="text-sm text-muted-foreground font-mono"
                  >
                    {entry.timestamp}
                  </time>
                  <span className="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground capitalize">
                    {entry.action}
                  </span>
                </div>
                {entry.details && (
                  <p className="text-sm text-muted-foreground">{entry.details}</p>
                )}
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
