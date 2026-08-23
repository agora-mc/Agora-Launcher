import { useState, useCallback, useEffect, useRef } from 'react';
import {
  checkInstanceHealth,
  disableModForTest,
  type HealthReport,
  type LoaderCompatibilityIssue,
  type RequirementVerdict,
} from '@/lib/tauri';
import {
  activeHealthWarnings,
  healthWarningKey,
  loadHealthPreferences,
  saveHealthPreferences,
  type HealthPreferences,
} from '@/lib/healthPreferences';
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
} from '@/components/ui/select';

interface HealthDialogProps {
  instanceId: string;
  instanceName: string;
  initialReport: HealthReport;
  /** User decided to launch despite warnings/blockers. Parent performs the actual launch. */
  onConfirm?: () => Promise<string | null>;
  /** User cancelled or closed the dialog. No launch. */
  onCancel: () => void;
  /** Switch the instance loader to a signed catalog version, then retry
   * launch in one backend operation. Rejects with an error message on
   * failure so the dialog can keep the error visible. */
  onSwitchLoader: (targetVersion: string) => Promise<HealthReport | void>;
  /** Opens the shared health UI outside a launch attempt. */
  reviewOnly?: boolean;
  /** Synchronizes a repair result with periodic instance-card health state. */
  onReportChanged?: (report: HealthReport) => void;
}

function verdictLabel(verdict: RequirementVerdict): string {
  if (verdict === 'satisfied') return 'Satisfied';
  if (verdict === 'unsatisfied') return 'Unsatisfied';
  return 'Unsupported';
}

function verdictKey(verdict: RequirementVerdict): string {
  return typeof verdict === 'string' ? verdict : `unsupported:${verdict.unsupported.reason}`;
}

interface RequirementGroup {
  targetId: string;
  versionRanges: string[];
  candidateVersion: string | null;
  verdict: RequirementVerdict;
  modIds: string[];
}

function groupRequirements(issue: LoaderCompatibilityIssue, satisfied: boolean): RequirementGroup[] {
  const groups = new Map<string, RequirementGroup>();
  for (const requirement of issue.requirements) {
    const isSatisfied = requirement.verdict === 'satisfied';
    if (isSatisfied !== satisfied) continue;
    const key = [
      requirement.target_id,
      requirement.version_ranges.join('\u0000'),
      requirement.candidate_version ?? '',
      verdictKey(requirement.verdict),
    ].join('\u0001');
    const group = groups.get(key) ?? {
      targetId: requirement.target_id,
      versionRanges: requirement.version_ranges,
      candidateVersion: requirement.candidate_version,
      verdict: requirement.verdict,
      modIds: [],
    };
    const modIds = requirement.declaring_mod_ids ?? (
      requirement.declaring_mod_id ? [requirement.declaring_mod_id] : []
    );
    for (const modId of modIds) {
      if (!group.modIds.includes(modId)) group.modIds.push(modId);
    }
    group.modIds.sort();
    groups.set(key, group);
  }
  return [...groups.values()].sort((left, right) => left.targetId.localeCompare(right.targetId));
}

function RequirementGroupCard({ group }: { group: RequirementGroup }) {
  const predicate = group.versionRanges.length > 0
    ? group.versionRanges.join(' or ')
    : 'any version';
  const affectedLabel = group.modIds.length === 1
    ? '1 affected mod'
    : `${group.modIds.length} affected mods`;
  return (
    <div className="rounded border border-border bg-background/60 p-2">
      <p className="text-xs font-medium">
        {group.targetId} {predicate}
        <span className="ml-1 text-muted-foreground">({verdictLabel(group.verdict)})</span>
      </p>
      {group.candidateVersion && (
        <p className="text-xs text-muted-foreground">
          Current loader provides {group.candidateVersion}
        </p>
      )}
      {group.modIds.length > 0 && (
        <details className="mt-1">
          <summary className="cursor-pointer select-none text-xs text-muted-foreground">
            {affectedLabel}
          </summary>
          <p className="mt-1 break-words text-xs text-muted-foreground">
            {group.modIds.join(', ')}
          </p>
        </details>
      )}
    </div>
  );
}

/**
 * Specialized loader repair card rendered when a health finding carries a
 * structured `loader_compatibility` payload. All decisions come from the
 * structured payload — the human-readable finding message is never parsed.
 */
function LoaderCompatibilityCard({
  issue,
  onSwitchLoader,
  onReportChanged,
  switchLabel,
  inProgress,
  tone,
}: {
  issue: LoaderCompatibilityIssue;
  onSwitchLoader: (targetVersion: string) => Promise<HealthReport | void>;
  onReportChanged?: (report: HealthReport) => void;
  switchLabel: string;
  inProgress: boolean;
  tone: 'blocker' | 'warning';
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const indeterminateVersions = issue.indeterminate_versions ?? [];
  const unresolvedRequirements = groupRequirements(issue, false);
  const satisfiedRequirements = groupRequirements(issue, true);
  const switchableCandidates = issue.compatible_versions.filter(
    (version) => version !== issue.current_version,
  );
  const hasCandidates = switchableCandidates.length > 0;
  const recommended = issue.recommended_version !== issue.current_version
    ? issue.recommended_version
    : null;
  const currentLabel = issue.current_version
    ? `${issue.loader} ${issue.current_version}`
    : issue.loader;

  const runSwitch = async (version: string) => {
    if (busy || inProgress) return;
    setBusy(true);
    setError(null);
    try {
      const nextReport = await onSwitchLoader(version);
      if (nextReport) onReportChanged?.(nextReport);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const containerClass =
    tone === 'blocker'
      ? 'border-destructive bg-destructive/10'
      : 'border-amber-500 bg-amber-500/10';
  const disabled = busy || inProgress;

  return (
    <div className={`rounded border p-2 text-sm ${containerClass}`}>
      <p className="font-semibold">
        Loader: {currentLabel}
      </p>
      {recommended ? (
        <p className="mt-1 text-xs text-muted-foreground">
          Recommended version: {recommended}
        </p>
      ) : hasCandidates ? (
        <p className="mt-1 text-xs text-muted-foreground">
          Compatible versions are available below.
        </p>
      ) : (
        <p className="mt-1 text-xs text-destructive">
          No signed loader version satisfies every enabled mod requirement. Review the
          conflicting requirements below; automatic switching is not available.
          {indeterminateVersions.length > 0
            ? ` ${indeterminateVersions.length} signed version${indeterminateVersions.length > 1 ? 's are' : ' is'} available for manual confirmation in the instance editor.`
            : ''}
        </p>
      )}

      {error && <p className="mt-1 text-xs text-destructive">{error}</p>}

      {hasCandidates && (
        <div className="mt-2 flex flex-wrap items-center gap-2">
          {recommended && (
            <Button
              size="sm"
              disabled={disabled}
              onClick={() => void runSwitch(recommended)}
            >
              {busy ? 'Switching…' : switchLabel}
            </Button>
          )}
          <Select
            onValueChange={(version) => void runSwitch(version)}
            disabled={disabled}
          >
            <SelectTrigger
              className="h-8 w-auto min-w-[12rem] text-xs"
              aria-label="Choose compatible version"
            >
              <span className="text-muted-foreground">Choose compatible version</span>
            </SelectTrigger>
            <SelectContent>
              {switchableCandidates.map((version) => (
                <SelectItem key={version} value={version}>
                  {version}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}

      {unresolvedRequirements.length > 0 && (
        <div className="mt-2 space-y-2">
          <p className="text-xs font-medium text-destructive">
            Unresolved requirements ({unresolvedRequirements.length})
          </p>
          {unresolvedRequirements.map((group, index) => (
            <RequirementGroupCard key={`${group.targetId}-${index}`} group={group} />
          ))}
        </div>
      )}

      {(issue.conflicts.length > 0 || indeterminateVersions.length > 0 || satisfiedRequirements.length > 0) && (
        <details className="mt-2">
          <summary className="cursor-pointer select-none text-xs font-medium">
            View compatibility evidence
            {satisfiedRequirements.length > 0 ? ` (${satisfiedRequirements.length} satisfied)` : ''}
            {issue.conflicts.length > 0 ? ` · ${issue.conflicts.length} conflict${issue.conflicts.length > 1 ? 's' : ''}` : ''}
          </summary>
          <div className="mt-2 space-y-2">
            {indeterminateVersions.length > 0 && (
              <p className="text-xs text-amber-600 dark:text-amber-400">
                Manual candidates: {indeterminateVersions.join(', ')}. These versions
                require explicit confirmation because at least one capability is unverified.
              </p>
            )}
            {satisfiedRequirements.map((group, index) => (
              <RequirementGroupCard key={`${group.targetId}-${index}`} group={group} />
            ))}
            {issue.conflicts.map((conflict, index) => (
              <div key={`conflict-${index}`} className="rounded border border-destructive bg-background/60 p-2">
                <p className="text-xs font-medium">Conflict</p>
                <p className="text-xs text-muted-foreground">{conflict.message}</p>
                <p className="text-xs text-muted-foreground">
                  {conflict.declaring_mod_id ?? 'unowned'} requires {conflict.target_id}{' '}
                  {conflict.version_ranges.join(' or ')} · {conflict.with_declaring_mod_id ?? 'unowned'}{' '}
                  requires {conflict.with_target_id} {conflict.with_version_ranges.join(' or ')}
                </p>
              </div>
            ))}
          </div>
        </details>
      )}
    </div>
  );
}

export function HealthDialog({
  instanceId,
  instanceName,
  initialReport,
  onConfirm,
  onCancel,
  onSwitchLoader,
  reviewOnly = false,
  onReportChanged,
}: HealthDialogProps) {
  const [report, setReport] = useState<HealthReport>(initialReport);
  const [error, setError] = useState<string | null>(null);
  const [preferences, setPreferences] = useState<HealthPreferences>({
    mutedWarnings: [],
    muteAllRecommendations: false,
  });
  const [fixing, setFixing] = useState<string | null>(null);
  const [launching, setLaunching] = useState(false);
  const [confirmingBlockers, setConfirmingBlockers] = useState(false);

  // Scroll container ref — reset to top whenever the report changes so a long
  // list of findings always starts at the top instead of clipping out of view.
  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [report]);

  const replaceReport = useCallback((nextReport: HealthReport) => {
    setReport(nextReport);
    onReportChanged?.(nextReport);
  }, [onReportChanged]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const loaded = await loadHealthPreferences(initialReport.warnings);
        if (!cancelled) setPreferences(loaded);
      } catch (e: any) {
        if (!cancelled) setError(e?.message || String(e));
      }
    })();
    return () => { cancelled = true; };
  }, [initialReport]);

  const handleFixDisable = useCallback(async (filename: string, key: string) => {
    // SOL-2 §18.5: reviewOnly is the approved health INSPECTION surface only.
    // The direct disable repair is a rejected seam (no dependency-aware plan,
    // recovery point, grouped rollback, or authoritative group outcome) and is
    // never reachable from a review-mode dialog.
    if (reviewOnly) return;
    setFixing(key);
    try {
      await disableModForTest(instanceId, filename);
      const r = await checkInstanceHealth(instanceId);
      replaceReport(r);
    } catch (e: any) {
      setError(e?.message || String(e));
    } finally {
      setFixing(null);
    }
  }, [reviewOnly, instanceId, replaceReport]);

  const handleSilence = useCallback(async (key: string, isMuted: boolean) => {
    const muted = new Set(preferences.mutedWarnings);
    if (isMuted) muted.delete(key); else muted.add(key);
    const next = { ...preferences, mutedWarnings: [...muted] };
    try {
      await saveHealthPreferences(next);
      setPreferences(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [preferences]);

  const handleRecommendationMute = useCallback(async (muted: boolean) => {
    const next = { ...preferences, muteAllRecommendations: muted };
    try {
      await saveHealthPreferences(next);
      setPreferences(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [preferences]);

  const handleConfirm = async () => {
    if (!onConfirm) return;
    setLaunching(true);
    setError(null);
    try {
      const launchError = await onConfirm();
      if (launchError) setError(launchError);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLaunching(false);
    }
  };

  const activeBlockers = report.blockers;
  const activeWarnings = activeHealthWarnings(report.warnings, preferences);
  const mutedWarnings = new Set(preferences.mutedWarnings);
  const recommendations = report.recommendations ?? [];

  const effectiveScore = activeBlockers.length > 0 ? 'red' as const
    : activeWarnings.length > 0 ? 'yellow' as const
    : 'green' as const;

  const scoreColors = {
    green: { bg: 'bg-green-500/10', border: 'border-green-500', text: 'text-green-600 dark:text-green-400', label: 'Ready to Launch' },
    yellow: { bg: 'bg-amber-500/10', border: 'border-amber-500', text: 'text-amber-600 dark:text-amber-400', label: 'Warnings' },
    red: { bg: 'bg-destructive/10', border: 'border-destructive', text: 'text-destructive', label: 'Launch Blocked' },
  } as const;

  const sc = scoreColors[effectiveScore];
  const requestLaunch = () => {
    if (activeBlockers.length > 0) {
      setConfirmingBlockers(true);
      return;
    }
    void handleConfirm();
  };

  const confirmBlockedLaunch = () => {
    setConfirmingBlockers(false);
    void handleConfirm();
  };

  return (
    <>
      <Dialog open onOpenChange={(open) => { if (!open && !launching) onCancel(); }}>
        <DialogContent className="max-w-lg max-h-[85vh] flex flex-col gap-3">
        <DialogTitle>Health Check</DialogTitle>
        <DialogDescription>
          {reviewOnly
            ? `${instanceName} has health findings that need review or repair.`
            : `${instanceName} has warnings or blockers that should be reviewed before launch.`}
        </DialogDescription>

        {/* Score badge */}
        <div className={`rounded-lg border ${sc.border} ${sc.bg} p-3`}>
          <div className="flex items-center gap-2">
            <div className={`text-lg font-bold ${sc.text}`}>
              {effectiveScore === 'green' ? '●' : effectiveScore === 'yellow' ? '◐' : '○'}
            </div>
            <div>
              <p className={`font-semibold ${sc.text}`}>{sc.label}</p>
              <p className="text-xs text-muted-foreground">{instanceName}</p>
            </div>
          </div>
        </div>

        {/* Scrollable findings region — blockers + warnings.
            A long list scrolls internally instead of clipping out of view;
            the dialog caps at 85vh and the actions footer stays pinned. */}
        <div
          ref={scrollRef}
          className="flex-1 min-h-0 overflow-y-auto pr-1 -mr-1"
        >
          {/* Blockers */}
          {report.blockers.length > 0 && (
            <div className="mb-3">
              <h4 className="text-sm font-semibold mb-2">Blockers ({report.blockers.length})</h4>
              <div className="space-y-2">
                {report.blockers.map((b, i) => {
                  const key = `${b.kind}:${b.mod_id ?? b.filename ?? i}`;
                  if (b.loader_compatibility) {
                    return (
                      <LoaderCompatibilityCard
                        key={key}
                        issue={b.loader_compatibility}
                        onSwitchLoader={onSwitchLoader}
                        onReportChanged={replaceReport}
                        switchLabel={reviewOnly ? 'Switch version' : 'Switch and launch'}
                        inProgress={launching || fixing !== null}
                        tone="blocker"
                      />
                    );
                  }
                  return (
                    <div key={key} className="rounded border border-destructive bg-destructive/10 p-2 text-sm">
                      <p className="text-destructive">{b.message}</p>
                      <div className="flex items-center gap-2 mt-1">
                        {b.suggested_action && (
                          <span className="text-xs text-muted-foreground">{b.suggested_action}</span>
                        )}
                      </div>
                      <div className="flex items-center gap-3 mt-2">
                        {!reviewOnly && (b.kind === 'missing_required_dependency' || b.kind === 'incompatible_mod' || b.kind === 'curated_conflict') && b.filename && (
                          <Button
                            size="sm"
                            variant="outline"
                            disabled={fixing === key}
                            title="Disable this mod and scan health again"
                            onClick={() => handleFixDisable(b.filename!, key)}
                          >
                            {fixing === key ? 'Fixing...' : 'Disable'}
                          </Button>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {/* Warnings */}
          {report.warnings.length > 0 && (
            <div className="mb-3">
              <h4 className="text-sm font-semibold mb-2">Warnings ({report.warnings.length})</h4>
              <div className="space-y-2">
                {report.warnings.map((w, i) => {
                  const key = healthWarningKey(w);
                  const isSilenced = mutedWarnings.has(key);
                  if (w.loader_compatibility) {
                    return (
                      <div key={i} className={`rounded border p-2 text-sm ${isSilenced ? 'border-border bg-muted/50 opacity-60' : ''}`}>
                        <LoaderCompatibilityCard
                          issue={w.loader_compatibility}
                          onSwitchLoader={onSwitchLoader}
                          onReportChanged={replaceReport}
                          switchLabel={reviewOnly ? 'Switch version' : 'Switch and launch'}
                          inProgress={launching || fixing !== null}
                          tone="warning"
                        />
                        <div className="flex items-center gap-2 text-xs mt-2">
                          <Switch
                            checked={!isSilenced}
                            onCheckedChange={() => handleSilence(key, isSilenced)}
                            aria-label="Show this warning"
                          />
                          <span className="text-muted-foreground">
                            {isSilenced ? 'Muted' : 'Show on next launch'}
                          </span>
                        </div>
                      </div>
                    );
                  }
                  return (
                    <div key={i} className={`rounded border p-2 text-sm ${isSilenced ? 'border-border bg-muted/50 opacity-60' : 'border-amber-500 bg-amber-500/10'}`}>
                      <p className={isSilenced ? 'text-muted-foreground line-through' : 'text-amber-600 dark:text-amber-400'}>{w.message}</p>
                      <div className="mt-2 space-y-2">
                        {w.suggested_action && !isSilenced && (
                          <p className="text-xs text-muted-foreground">{w.suggested_action}</p>
                        )}
                        <div className="flex items-center justify-between gap-3">
                          <div className="flex shrink-0 items-center gap-2 text-xs">
                            <Switch
                              checked={!isSilenced}
                              onCheckedChange={() => handleSilence(key, isSilenced)}
                              aria-label="Show this warning"
                            />
                            <span className="text-muted-foreground">
                              {isSilenced ? 'Muted' : 'Show on next launch'}
                            </span>
                          </div>
                          {!reviewOnly && (w.kind === 'missing_required_dependency_unverified' || w.kind === 'incompatible_mod_unverified' || w.kind === 'incompatible_mod_soft' || w.kind === 'curated_conflict_soft') && w.filename && (
                            <Button
                              size="sm"
                              variant="outline"
                              disabled={fixing === key}
                              title="Disable this mod and scan health again"
                              onClick={() => handleFixDisable(w.filename!, key)}
                            >
                              {fixing === key ? 'Fixing...' : 'Disable'}
                            </Button>
                          )}
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {recommendations.length > 0 && (
            <details className="mb-3 rounded border border-border bg-muted/30 p-2">
              <summary className="cursor-pointer select-none text-sm font-semibold">
                Recommendations ({recommendations.length})
                {preferences.muteAllRecommendations ? ' - muted' : ''}
              </summary>
              <div className="mt-3 space-y-3">
                <div className="flex items-center justify-between rounded border border-border bg-background p-2">
                  <div>
                    <p className="text-sm font-medium">Show recommendations</p>
                    <p className="text-xs text-muted-foreground">Optional improvements never block launch.</p>
                  </div>
                  <Switch
                    checked={!preferences.muteAllRecommendations}
                    onCheckedChange={(checked) => handleRecommendationMute(!checked)}
                    aria-label="Show health recommendations"
                  />
                </div>
                {!preferences.muteAllRecommendations && recommendations.map((recommendation, index) => (
                  <div key={`${recommendation.kind}:${recommendation.mod_id ?? index}`} className="rounded border border-border bg-background p-2 text-sm">
                    <p>{recommendation.message}</p>
                    {recommendation.suggested_action && (
                      <p className="mt-1 text-xs text-muted-foreground">{recommendation.suggested_action}</p>
                    )}
                  </div>
                ))}
              </div>
            </details>
          )}

          {/* All clear */}
          {effectiveScore === 'green' && (
            <p className="text-sm text-green-600 dark:text-green-400 mb-4">
              No issues found. Instance is ready to launch.
            </p>
          )}
        </div>

        {/* Actions */}
        {error && <p className="text-sm text-destructive">{error}</p>}
        <div className="flex gap-2 justify-end pt-3 border-t border-border">
          <Button variant="outline" onClick={onCancel} disabled={launching}>{reviewOnly ? 'Close' : 'Cancel'}</Button>
          {reviewOnly ? null : (
            <Button
              variant={activeBlockers.length > 0 ? 'destructive' : 'default'}
              onClick={requestLaunch}
              disabled={launching}
            >
              {launching ? 'Launching…' : 'Launch Anyway'}
            </Button>
          )}
        </div>
        </DialogContent>
      </Dialog>

      <Dialog open={confirmingBlockers} onOpenChange={setConfirmingBlockers}>
        <DialogContent className="z-[60] max-w-md gap-4">
          <DialogTitle>Launch despite blockers?</DialogTitle>
          <DialogDescription>
            Agora found {activeBlockers.length} blocker{activeBlockers.length === 1 ? '' : 's'}.
            Continuing may cause a failed launch, crash, or unstable gameplay.
            Your instance could be functional, but Agora cannot guarantee it. If you are unsure, review the findings first.
            If this is a downloaded modpack, the pack may install additional content or make other changes at launch time, which Agora cannot predict.
          </DialogDescription>
          <div role="alert" className="rounded border border-destructive bg-destructive/10 p-3 text-sm">
            <p className="font-medium text-destructive">The runtime loader will still validate this mod set.</p>
            <p className="mt-1 text-muted-foreground">
              Agora will not change mods or loader settings. Review the findings first if you want to repair them.
            </p>
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={() => setConfirmingBlockers(false)}>
              Review Blockers
            </Button>
            <Button variant="destructive" onClick={confirmBlockedLaunch}>
              I Understand, Launch Anyway
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
