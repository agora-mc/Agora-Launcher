import { useState, useCallback, useEffect, useRef } from 'react';
import {
  checkInstanceHealth,
  disableModForTest,
  type HealthReport,
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

interface HealthDialogProps {
  instanceId: string;
  instanceName: string;
  initialReport: HealthReport;
  /** User decided to launch despite warnings/blockers. Parent performs the actual launch. */
  onConfirm: () => Promise<string | null>;
  /** User cancelled or closed the dialog. No launch. */
  onCancel: () => void;
}

export function HealthDialog({ instanceId, instanceName, initialReport, onConfirm, onCancel }: HealthDialogProps) {
  const [report, setReport] = useState<HealthReport>(initialReport);
  const [error, setError] = useState<string | null>(null);
  const [preferences, setPreferences] = useState<HealthPreferences>({
    mutedWarnings: [],
    muteAllRecommendations: false,
  });
  const [fixing, setFixing] = useState<string | null>(null);
  const [launching, setLaunching] = useState(false);

  // Scroll container ref — reset to top whenever the report changes so a long
  // list of findings always starts at the top instead of clipping out of view.
  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [report]);

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
    setFixing(key);
    try {
      await disableModForTest(instanceId, filename);
      const r = await checkInstanceHealth(instanceId);
      setReport(r);
    } catch (e: any) {
      setError(e?.message || String(e));
    } finally {
      setFixing(null);
    }
  }, [instanceId]);

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

  return (
    <Dialog open onOpenChange={(open) => { if (!open && !launching) onCancel(); }}>
      <DialogContent className="max-w-lg max-h-[85vh] flex flex-col gap-3">
        <DialogTitle>Health Check</DialogTitle>
        <DialogDescription>
          {instanceName} has warnings or blockers that should be reviewed before launch.
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
                  return (
                    <div key={key} className="rounded border border-destructive bg-destructive/10 p-2 text-sm">
                      <p className="text-destructive">{b.message}</p>
                      <div className="flex items-center gap-2 mt-1">
                        {b.suggested_action && (
                          <span className="text-xs text-muted-foreground">{b.suggested_action}</span>
                        )}
                      </div>
                      <div className="flex items-center gap-3 mt-2">
                        {(b.kind === 'missing_required_dependency' || b.kind === 'incompatible_mod' || b.kind === 'curated_conflict') && b.filename && (
                          <Button
                            size="sm"
                            variant="outline"
                            disabled={fixing === key}
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
                  return (
                    <div key={i} className={`rounded border p-2 text-sm ${isSilenced ? 'border-border bg-muted/50 opacity-60' : 'border-amber-500 bg-amber-500/10'}`}>
                      <p className={isSilenced ? 'text-muted-foreground line-through' : 'text-amber-600 dark:text-amber-400'}>{w.message}</p>
                      <div className="flex items-center justify-between mt-2">
                        <div className="flex items-center gap-2 text-xs">
                          <Switch
                            checked={!isSilenced}
                            onCheckedChange={() => handleSilence(key, isSilenced)}
                            aria-label="Show this warning"
                          />
                          <span className="text-muted-foreground">
                            {isSilenced ? 'Muted' : 'Show on next launch'}
                          </span>
                        </div>
                        {w.suggested_action && !isSilenced && (
                          <span className="text-xs text-muted-foreground">{w.suggested_action}</span>
                        )}
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
          <Button variant="outline" onClick={onCancel} disabled={launching}>Cancel</Button>
          {activeBlockers.length > 0 ? (
            <Button variant="destructive" disabled>
              Resolve {activeBlockers.length} blocker{activeBlockers.length > 1 ? 's' : ''}
            </Button>
          ) : (
            <Button onClick={handleConfirm} disabled={launching}>{launching ? 'Launching…' : 'Launch Anyway'}</Button>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
