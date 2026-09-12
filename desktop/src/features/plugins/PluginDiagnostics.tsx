import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { formatError } from '@/lib/tauri';
import { applyPluginRepair, runPluginDiagnostic } from './api';
import type { DiagnosticReport, RepairAction, RepairProposal } from './types';

export function describeRepair(action: RepairAction): string {
  const target = `in instance ${action.instanceId}`;
  switch (action.action) {
    case 'disableContent': return `Disable ${action.key} ${target}`;
    case 'enableContent': return `Enable ${action.key} ${target}`;
    case 'pinContentUpdate': return `Pin updates for ${action.key} ${target}`;
    case 'unpinContentUpdate': return `Resume updates for ${action.key} ${target}`;
    case 'setJvmMemory': return `Set Java memory to ${action.memoryMb} MB ${target}`;
    case 'resetJvmArgs': return `Reset custom Java arguments ${target}`;
    case 'createSnapshot': return `Create a snapshot ${target}`;
  }
}

export function PluginDiagnostic({ pluginId, exportName, title, instanceId }: {
  pluginId: string; exportName: string; title: string; instanceId: string;
}) {
  const [report, setReport] = useState<DiagnosticReport | null>(null);
  const [proposal, setProposal] = useState<RepairProposal | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const run = async () => {
    setBusy(true); setMessage(''); setReport(null);
    try { setReport(await runPluginDiagnostic(pluginId, exportName, instanceId)); }
    catch (error) { setMessage(formatError(error)); }
    finally { setBusy(false); }
  };
  return <section className="space-y-3 rounded-lg border border-border p-4">
    <Button variant="secondary" disabled={busy} onClick={() => void run()}>{busy ? 'Working…' : title}</Button>
    <p className="text-xs text-muted-foreground">Provided by {pluginId}</p>
    {message && <p role="status">{message}</p>}
    {report?.incompleteReason && <p role="alert">Check incomplete: {report.incompleteReason}</p>}
    {report && !report.incompleteReason && report.findings.length === 0 && <p>No findings reported.</p>}
    {report?.findings.map((finding) => <div key={finding.id} className="space-y-2 border-t border-border pt-3">
      <h4 className="font-medium">{finding.title} · {finding.severity}</h4>
      {finding.summary && <p>{finding.summary}</p>}
      <ul>{finding.evidence.map((evidence, index) => <li key={index}>{evidence.label}: {evidence.value}</li>)}</ul>
      {finding.repairs.map((repair) => <Button key={repair.id} variant="secondary" disabled={busy}
        onClick={() => setProposal(repair)}>Review: {repair.title}</Button>)}
    </div>)}
    <Dialog open={proposal !== null} onOpenChange={(open) => { if (!open && !busy) setProposal(null); }}>
      <DialogContent><DialogTitle>Review plugin repair</DialogTitle>
        <DialogDescription>{pluginId} proposes these changes. Agora checks the current state before applying them.</DialogDescription>
        <ul className="list-disc pl-5">{proposal?.actions.map((action, index) => <li key={index}>{describeRepair(action)}</li>)}</ul>
        <Button disabled={busy} onClick={() => {
          if (!proposal) return;
          setBusy(true);
          void applyPluginRepair(pluginId, proposal).then((outcome) => {
            setMessage(`${outcome.applied.length} applied. ${outcome.failed.length} failed. ${outcome.stale.length} stale. ` +
              [...outcome.failed.map((failure) => failure.message), ...outcome.stale].join(' '));
            setProposal(null); setReport(null);
          }).catch((error: unknown) => setMessage(formatError(error))).finally(() => setBusy(false));
        }}>Apply reviewed changes</Button>
        <Button variant="secondary" disabled={busy} onClick={() => setProposal(null)}>Cancel</Button>
      </DialogContent>
    </Dialog>
  </section>;
}
