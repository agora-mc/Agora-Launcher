import { useCallback, useEffect, useRef, useState } from 'react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { formatError } from '@/lib/tauri';
import { runPluginLaunchChecks } from './api';
import { usePlugins } from './PluginProvider';
import type { LaunchCheckOutcome } from './types';

export function usePluginLaunchGate() {
  const { ofKind } = usePlugins();
  const hasChecks = ofKind('launch-check').length > 0;
  const [outcome, setOutcome] = useState<LaunchCheckOutcome | null>(null);
  const pending = useRef<((proceed: boolean) => void) | null>(null);
  const finish = useCallback((proceed: boolean) => {
    pending.current?.(proceed); pending.current = null; setOutcome(null);
  }, []);
  useEffect(() => () => { pending.current?.(false); pending.current = null; }, []);
  const check = useCallback(async (instanceId: string) => {
    if (!hasChecks) return true;
    if (pending.current) return false;
    let result: LaunchCheckOutcome;
    try { result = await runPluginLaunchChecks(instanceId); }
    catch (error) { result = { results: [], errors: [formatError(error)] }; }
    if (!result.errors.length && result.results.every((entry) => !entry.report.findings.length && !entry.report.incompleteReason)) return true;
    return new Promise<boolean>((resolve) => { pending.current = resolve; setOutcome(result); });
  }, [hasChecks]);
  const dialog = <Dialog open={outcome !== null} onOpenChange={(open) => { if (!open) finish(false); }}>
    <DialogContent><DialogTitle>Plugin launch checks</DialogTitle>
      <DialogDescription>Review these optional plugin findings before continuing to Agora’s normal launch checks.</DialogDescription>
      <div className="max-h-80 space-y-3 overflow-auto">
        {outcome?.errors.map((error, index) => <p key={index} role="alert">Check failed: {error}</p>)}
        {outcome?.results.map((result) => <section key={result.checkId}>
          <h3 className="font-semibold">{result.title} ({result.pluginId})</h3>
          {result.report.incompleteReason && <p>Incomplete: {result.report.incompleteReason}</p>}
          {result.report.findings.map((finding) => <p key={finding.id}>{finding.severity}: {finding.title}. {finding.summary}</p>)}
        </section>)}
      </div>
      <Button onClick={() => finish(true)}>Continue to launch checks</Button>
      <Button variant="secondary" onClick={() => finish(false)}>Cancel launch</Button>
    </DialogContent>
  </Dialog>;
  return { check, dialog };
}
