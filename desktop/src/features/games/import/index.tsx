/**
 * Import slot owned by the import slice: the Import tab (Mod Organizer 2 setups).
 *
 * Find a setup (the usual places and the drives, or a folder the user picks), choose a profile,
 * preview what an import would do, then import it with progress. The setup is only read.
 */

import { useEffect, useRef, useState } from 'react';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { Download, FolderOpen, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useConfirm } from '@/components/ui/confirm';
import { formatBytes } from '@/components/installed-content/contentTableState';
import { formatError, pickDirectory } from '@/lib/tauri';
import {
  importMo2Profile,
  listenImportProgress,
  scanMo2Setups,
  type ImportProgress,
  type Mo2PlanDto,
  type Mo2RunDto,
  type SetupFound,
} from './api';
import { planBlockReason } from './planRules';
import { PlanView } from './PlanView';

type ScanState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'loaded'; setups: SetupFound[]; warnings: string[] };

type PlanState =
  | { status: 'idle' }
  | { status: 'busy' }
  | { status: 'error'; message: string }
  | { status: 'ready'; plan: Mo2PlanDto };

type RunState =
  | { status: 'idle' }
  | { status: 'running'; progress: ImportProgress | null }
  | { status: 'error'; message: string }
  | { status: 'done'; report: Mo2RunDto };

export function GameImportPage({ gameId }: { gameId: string }) {
  const { confirm } = useConfirm();
  const [scan, setScan]= useState<ScanState>({ status: 'loading' });
  const [pickError, setPickError] = useState<string | null>(null);
  const [chosen, setChosen] = useState<SetupFound | null>(null);
  const [profile, setProfile] = useState('');
  const [instanceName, setInstanceName] = useState('');
  const [copySaves, setCopySaves] = useState(false);
  const [plan, setPlan] = useState<PlanState>({ status: 'idle' });
  const [run, setRun] = useState<RunState>({ status: 'idle' });
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const loadSetups = () => {
    setScan({ status: 'loading' });
    scanMo2Setups()
      .then((reply) => {
        if (mounted.current) setScan({ status: 'loaded', setups: reply.setups, warnings: reply.warnings });
      })
      .catch((error: unknown) => {
        if (mounted.current) setScan({ status: 'error', message: formatError(error) });
      });
  };

  useEffect(() => {
    loadSetups();
    // Looked up when the tab opens; "Look again" looks again.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const choose = (setup: SetupFound) => {
    setChosen(setup);
    setProfile(setup.profiles[0] ?? '');
    setInstanceName('');
    setCopySaves(false);
    setPlan({ status: 'idle' });
    setRun({ status: 'idle' });
  };

  const pickFolder = async () => {
    setPickError(null);
    const folder = await pickDirectory('Choose the folder that holds ModOrganizer.ini').catch(() => null);
    if (!folder) return;
    try {
      const reply = await scanMo2Setups(folder);
      const [only] = reply.setups;
      if (!only) {
        setPickError('No setup was found in that folder.');
        return;
      }
      choose(only);
    } catch (error) {
      setPickError(formatError(error));
    }
  };

  const request = (dryRun: boolean) => ({
    setupPath: chosen?.iniPath ?? '',
    profile,
    name: instanceName.trim() === '' ? null : instanceName.trim(),
    copySaves,
    dryRun,
  });

  const previewImport = async () => {
    setPlan({ status: 'busy' });
    try {
      const reply = await importMo2Profile(request(true));
      if (reply.kind !== 'plan') throw new Error('The preview did not return a plan.');
      if (mounted.current) setPlan({ status: 'ready', plan: reply });
    } catch (error) {
      if (mounted.current) setPlan({ status: 'error', message: formatError(error) });
    }
  };

  const runImport = async (current: Mo2PlanDto) => {
    const ok = await confirm({
      title: `Import “${current.profile}” as a new instance?`,
      body: `Agora copies ${current.mods.length} mods (${formatBytes(current.bytes)}) into its content store and makes an instance named “${current.instanceName}”. The Mod Organizer 2 setup is not changed.`,
      confirmLabel: 'Import profile',
    });
    if (!ok) return;
    setRun({ status: 'running', progress: null });
    let unlisten: UnlistenFn | null = null;
    try {
      unlisten = await listenImportProgress((progress) => {
        if (mounted.current) setRun({ status: 'running', progress });
      });
      const reply = await importMo2Profile(request(false));
      if (reply.kind !== 'run') throw new Error('The import did not return a report.');
      if (mounted.current) setRun({ status: 'done', report: reply });
    } catch (error) {
      if (mounted.current) setRun({ status: 'error', message: formatError(error) });
    } finally {
      unlisten?.();
    }
  };

  return (
    <section className="space-y-6" data-testid="import-page">
      <header className="space-y-1">
        <h2 className="text-lg font-semibold">Import from Mod Organizer 2</h2>
        <p className="text-sm text-muted-foreground">
          Bring a Mod Organizer 2 profile in as an Agora instance. Agora only reads the setup: every mod is copied into
          Agora as it is on disk, and the profile’s load order, plugins, INIs and saves are set on the new instance.
        </p>
      </header>

      <div className="space-y-3 rounded-xl border border-border bg-card p-5">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h3 className="text-base font-semibold">Setups on this computer</h3>
          <div className="flex gap-2">
            <Button variant="outline" size="sm" onClick={loadSetups} disabled={scan.status === 'loading'}>
              <RefreshCw className="mr-2 h-4 w-4" aria-hidden="true" />
              Look again
            </Button>
            <Button variant="outline" size="sm" onClick={pickFolder}>
              <FolderOpen className="mr-2 h-4 w-4" aria-hidden="true" />
              Choose a setup folder…
            </Button>
          </div>
        </div>

        {scan.status === 'loading' && (
          <p role="status" className="text-sm text-muted-foreground">
            Looking for Mod Organizer 2 setups. Agora checks the usual place and your drives, which can take a minute.
          </p>
        )}

        {scan.status === 'error' && (
          <p role="alert" className="text-sm text-destructive">
            Agora could not look for setups: {scan.message}
          </p>
        )}

        {scan.status === 'loaded' && scan.setups.length === 0 && (
          <p className="text-sm text-muted-foreground" data-testid="import-empty">
            No Mod Organizer 2 setups were found in the usual place or on your drives. If yours is somewhere else, choose
            the folder it is in.
          </p>
        )}

        {scan.status === 'loaded' && scan.setups.length > 0 && (
          <ul className="space-y-2" aria-label="Setups found">
            {scan.setups.map((setup) => (
              <li
                key={setup.iniPath}
                className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border p-3 text-sm"
              >
                <span className="min-w-0 break-all">
                  <span className="font-medium">{setup.gameName ?? 'Game not named'}</span>
                  <span className="block text-muted-foreground">
                    {setup.iniPath} · {setup.profiles.length} profile(s) · {setup.modCount ?? 'unreadable'} mod folder(s)
                    · found {setup.foundBy}
                  </span>
                </span>
                <Button
                  size="sm"
                  variant={chosen?.iniPath === setup.iniPath ? 'secondary' : 'default'}
                  onClick={() => choose(setup)}
                >
                  Use this setup
                </Button>
              </li>
            ))}
          </ul>
        )}

        {scan.status === 'loaded' &&
          scan.warnings.map((warning) => (
            <p key={warning} className="text-sm text-amber-700 dark:text-amber-300">
              {warning}
            </p>
          ))}

        {pickError && (
          <p role="alert" className="text-sm text-destructive">
            {pickError}
          </p>
        )}
      </div>

      {chosen && (
        <div className="space-y-4 rounded-xl border border-border bg-card p-5" data-testid="import-form">
          <h3 className="text-base font-semibold">Choose a profile</h3>
          {chosen.profiles.length === 0 ? (
            <p className="text-sm text-muted-foreground">This setup has no profiles, so there is nothing to import.</p>
          ) : (
            <>
              <label className="grid gap-1 text-sm">
                <span className="font-medium">Profile</span>
                <select
                  className="rounded-lg border border-border bg-background px-3 py-2"
                  value={profile}
                  onChange={(event) => {
                    setProfile(event.target.value);
                    setPlan({ status: 'idle' });
                    setRun({ status: 'idle' });
                  }}
                >
                  {chosen.profiles.map((name) => (
                    <option key={name} value={name}>
                      {name}
                    </option>
                  ))}
                </select>
              </label>

              <label className="grid gap-1 text-sm">
                <span className="font-medium">Instance name (optional)</span>
                <span className="text-muted-foreground">
                  Leave this empty to name it after the profile. Only needed to import the same profile a second time.
                </span>
                <input
                  className="rounded-lg border border-border bg-background px-3 py-2"
                  value={instanceName}
                  onChange={(event) => setInstanceName(event.target.value)}
                  placeholder={profile ? `${profile} (MO2 import)` : ''}
                />
              </label>

              <label className="flex items-start gap-3 text-sm">
                <input
                  type="checkbox"
                  className="mt-1"
                  checked={copySaves}
                  onChange={(event) => setCopySaves(event.target.checked)}
                />
                <span>
                  <span className="font-medium">Copy this profile’s saves into the new instance</span>
                  <span className="block text-muted-foreground">
                    Saves are copied into the instance’s own save folder under your Documents. Files that already exist
                    there are left alone. Off: the saves stay where they are, and you can copy them by hand.
                  </span>
                </span>
              </label>

              <div>
                <Button
                  onClick={previewImport}
                  disabled={!profile || plan.status === 'busy' || run.status === 'running'}
                >
                  {plan.status === 'busy' ? 'Reading the profile…' : 'Preview the import'}
                </Button>
              </div>

              {plan.status === 'error' && (
                <p role="alert" className="text-sm text-destructive">
                  The preview failed: {plan.message}
                </p>
              )}
            </>
          )}
        </div>
      )}

      {chosen && plan.status === 'ready' && (
        <PlanView
          plan={plan.plan}
          blockReason={planBlockReason(plan.plan, gameId)}
          busy={run.status === 'running'}
          onImport={() => runImport(plan.plan)}
        />
      )}

      {run.status === 'running' && (
        <p role="status" className="rounded-xl border border-border bg-card p-4 text-sm" data-testid="import-progress">
          {run.progress
            ? `Storing mods: ${run.progress.modsDone} of ${run.progress.modsTotal}, ${formatBytes(run.progress.bytesDone)} of ${formatBytes(run.progress.bytesTotal)}. A large setup takes a while; keep Agora open.`
            : 'Starting the import. Keep Agora open.'}
        </p>
      )}

      {run.status === 'error' && (
        <p role="alert" className="rounded-xl border border-destructive/40 bg-destructive/10 p-4 text-sm">
          The import did not finish: {run.message} Preview the import again to see what is still to do.
        </p>
      )}

      {run.status === 'done' && <RunSummary report={run.report} />}
    </section>
  );
}

function RunSummary({ report }: { report: Mo2RunDto }) {
  return (
    <section
      aria-labelledby="import-done-heading"
      className="space-y-3 rounded-xl border border-border bg-card p-5"
      data-testid="import-done"
    >
      <h3 id="import-done-heading" className="flex items-center gap-2 text-base font-semibold">
        <Download className="h-4 w-4" aria-hidden="true" />
        Imported as “{report.instanceName}”
      </h3>
      <p className="text-sm">
        {report.storedMods} mod(s) stored; {report.contentLayers} content layer(s), {report.disabledLayers} of them
        disabled.
        {report.baseId && (
          <>
            {' '}
            Base {report.baseId} {report.baseBuilt ? 'was built' : 'was reused'}.
          </>
        )}
      </p>
      {report.duplicates.length > 0 && (
        <p className="text-sm text-muted-foreground">Already in this instance (same bytes): {report.duplicates.join(', ')}</p>
      )}
      {report.generatedLayers.length > 0 && (
        <p className="text-sm text-muted-foreground">
          Generated layers (inputs unknown): {report.generatedLayers.join(', ')}
        </p>
      )}
      {report.pluginFindings.length > 0 && (
        <ul className="space-y-1 text-sm" aria-label="Plugin order findings">
          {report.pluginFindings.map((finding) => (
            <li key={finding}>{finding}</li>
          ))}
        </ul>
      )}
      {report.nextSteps.length > 0 && (
        <div>
          <h4 className="text-sm font-medium">Still to do</h4>
          <ul className="mt-1 space-y-1 text-sm">
            {report.nextSteps.map((step) => (
              <li key={step}>{step}</li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}
