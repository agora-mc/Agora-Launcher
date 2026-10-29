/**
 * The FOMOD wizard: an archive's installer questions, one page per step, then a preview of the
 * files the answers would install, then the install. Opened when an archive that has an installer
 * is added to an instance (the content slice mounts it), and by the Library catalog.
 *
 * The core decides everything that matters: which choices are allowed, what a step's visibility
 * means, and what gets installed. This component shows the core's answers and its refusals.
 */

import { useEffect, useState } from 'react';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { formatBytes } from '../content/api';
import { isGamesError } from '../shared/types';
import {
  installFomod,
  previewFomod,
  showFomod,
  type FomodChoice,
  type FomodGroupView,
  type FomodInstallerView,
  type FomodPlan,
  type FomodInstalled,
  type FomodPluginView,
} from './api';

export interface FomodWizardProps {
  /** The archive item (in the content store) whose installer runs. */
  contentId: string;
  /** The instance the result is added to. Omit it to install into the library only. */
  instanceId?: string | null;
  /** Close the wizard. The slot is unmounted by the caller. */
  onClose: () => void;
  /** Called once the install has succeeded. */
  onInstalled?: (installed: FomodInstalled) => void;
}

/** The user's picks: plugin names by step and group. */
type Selection = Record<string, string[]>;

type View =
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'options' }
  | { kind: 'review'; plan: FomodPlan }
  | { kind: 'installed'; installed: FomodInstalled };

function errorMessage(error: unknown): string {
  if (isGamesError(error)) return error.message;
  if (error instanceof Error) return error.message;
  return 'Something went wrong. Try again.';
}

function groupKey(step: string, group: string): string {
  return JSON.stringify([step, group]);
}

function selectionFrom(choices: FomodChoice[]): Selection {
  const selection: Selection = {};
  for (const choice of choices) {
    selection[groupKey(choice.step, choice.group)] = [...choice.plugins];
  }
  return selection;
}

/** The choices as the core takes them: every group the user has picked from, in installer order. */
function choicesFrom(installer: FomodInstallerView, selection: Selection): FomodChoice[] {
  const choices: FomodChoice[] = [];
  for (const step of installer.steps) {
    for (const group of step.groups) {
      const plugins = selection[groupKey(step.name, group.name)] ?? [];
      if (plugins.length > 0) choices.push({ step: step.name, group: group.name, plugins: [...plugins] });
    }
  }
  return choices;
}

function typeBadge(plugin: FomodPluginView): string | null {
  switch (plugin.pluginType) {
    case 'required':
      return 'Required';
    case 'recommended':
      return 'Recommended';
    case 'not_usable':
      return 'Cannot be used here';
    case 'dependent':
      return 'Depends on your other choices';
    case 'could_be_usable':
      return 'Might not work here';
    default:
      return null;
  }
}

export function FomodWizard({ contentId, instanceId = null, onClose, onInstalled }: FomodWizardProps) {
  const [view, setView] = useState<View>({ kind: 'loading' });
  const [installer, setInstaller] = useState<FomodInstallerView | null>(null);
  const [selection, setSelection] = useState<Selection>({});
  const [page, setPage] = useState(0);
  const [busy, setBusy] = useState(false);

  /** Read the installer, and take the proposed choices when asked (the "Use defaults" path). */
  const load = async (useDefaults: boolean) => {
    setView({ kind: 'loading' });
    try {
      const shown = await showFomod(contentId, instanceId, useDefaults);
      if (useDefaults) {
        if (shown.suggestedChoices === null) {
          throw new Error('Agora did not propose any choices for this installer.');
        }
        setSelection(selectionFrom(shown.suggestedChoices));
      }
      setInstaller(shown.installer);
      setPage(0);
      setView({ kind: 'options' });
    } catch (error) {
      setView({ kind: 'error', message: errorMessage(error) });
    }
  };

  useEffect(() => {
    void load(true);
    // The wizard reads one archive for its whole life; the caller mounts a new one for another.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [contentId, instanceId]);

  const toggle = (step: string, group: FomodGroupView, plugin: string) => {
    const key = groupKey(step, group.name);
    setSelection((current) => {
      const picked = current[key] ?? [];
      let next: string[];
      if (group.singleChoice) {
        next = picked.includes(plugin) ? [] : [plugin];
      } else {
        next = picked.includes(plugin) ? picked.filter((name) => name !== plugin) : [...picked, plugin];
      }
      return { ...current, [key]: next };
    });
  };

  const review = async () => {
    if (!installer) return;
    setBusy(true);
    try {
      const result = await previewFomod(contentId, instanceId, choicesFrom(installer, selection));
      if (result.status !== 'preview') throw new Error('Agora did not return a file plan.');
      setView({ kind: 'review', plan: result });
    } catch (error) {
      setView({ kind: 'error', message: errorMessage(error) });
    } finally {
      setBusy(false);
    }
  };

  const install = async () => {
    if (!installer) return;
    setBusy(true);
    try {
      const result = await installFomod(contentId, instanceId, choicesFrom(installer, selection));
      if (result.status !== 'installed') throw new Error('Agora did not confirm the install.');
      onInstalled?.(result);
      setView({ kind: 'installed', installed: result });
    } catch (error) {
      setView({ kind: 'error', message: errorMessage(error) });
    } finally {
      setBusy(false);
    }
  };

  const steps = installer?.steps ?? [];
  const lastPage = Math.max(steps.length - 1, 0);
  const title = installer?.title ?? 'Installer options';

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="max-w-2xl" data-testid="fomod-wizard">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {view.kind === 'review'
              ? 'Review the files this choice installs before anything is added.'
              : view.kind === 'installed'
                ? 'The installer has finished.'
                : 'Answer the installer’s questions. Required and recommended options are already chosen.'}
          </DialogDescription>
        </DialogHeader>

        {view.kind === 'loading' && <p className="text-sm text-muted-foreground">Reading the installer…</p>}

        {view.kind === 'error' && (
          <div role="alert" className="rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm">
            {view.message}
          </div>
        )}

        {view.kind === 'options' && installer && (
          <div className="space-y-4">
            {steps.length === 0 && (
              <p className="text-sm text-muted-foreground">This installer asks no questions. Review its files to install them.</p>
            )}
            {steps[page] && (
              <section aria-label={`Step ${page + 1} of ${steps.length}`} className="space-y-3">
                <h3 className="text-base font-semibold">
                  {steps[page].name} <span className="text-sm font-normal text-muted-foreground">(step {page + 1} of {steps.length})</span>
                </h3>
                {steps[page].visible && (
                  <p className="text-sm text-muted-foreground">This page applies only when {steps[page].visible}.</p>
                )}
                {steps[page].groups.map((group) => {
                  const picked = selection[groupKey(steps[page].name, group.name)] ?? [];
                  const inputType = group.singleChoice ? 'radio' : 'checkbox';
                  return (
                    <fieldset key={group.name} className="space-y-2 rounded-lg border border-border p-3">
                      <legend className="px-1 text-sm font-medium">
                        {group.name} <span className="font-normal text-muted-foreground">({group.rule})</span>
                      </legend>
                      {group.plugins.map((plugin) => {
                        const locked = plugin.pluginType === 'required' || plugin.pluginType === 'not_usable';
                        const badge = typeBadge(plugin);
                        return (
                          <label key={plugin.name} className="flex items-start gap-3 text-sm">
                            <input
                              type={inputType}
                              name={groupKey(steps[page].name, group.name)}
                              className="mt-1"
                              checked={picked.includes(plugin.name) || plugin.pluginType === 'required'}
                              disabled={locked}
                              onChange={() => toggle(steps[page].name, group, plugin.name)}
                            />
                            <span className="space-y-0.5">
                              <span className="font-medium">{plugin.name}</span>
                              {badge && <span className="ml-2 rounded bg-muted px-1.5 py-0.5 text-xs">{badge}</span>}
                              {plugin.description && (
                                <span className="block text-muted-foreground">{plugin.description}</span>
                              )}
                            </span>
                          </label>
                        );
                      })}
                    </fieldset>
                  );
                })}
              </section>
            )}
          </div>
        )}

        {view.kind === 'review' && (
          <div className="space-y-3 text-sm">
            <p>
              {view.plan.files.length === 0
                ? 'These choices install no files, so there is nothing to add.'
                : `${view.plan.files.length} file${view.plan.files.length === 1 ? '' : 's'} (${formatBytes(view.plan.totalSize)}), placed ${view.plan.dataFolder ? `in ${view.plan.dataFolder}` : 'at the top of the game folder'}:`}
            </p>
            {view.plan.files.length > 0 && (
              <ul className="max-h-64 space-y-1 overflow-auto rounded-lg border border-border p-3 font-mono text-xs" aria-label="Files to install">
                {view.plan.files.map((file) => (
                  <li key={file.destination} className="flex justify-between gap-4">
                    <span className="truncate">{file.destination}</span>
                    <span className="shrink-0 text-muted-foreground">{formatBytes(file.size)}</span>
                  </li>
                ))}
              </ul>
            )}
            {view.plan.notes.length > 0 && (
              <div className="space-y-1">
                <p className="font-medium">Things to know</p>
                <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
                  {view.plan.notes.map((note) => <li key={note}>{note}</li>)}
                </ul>
              </div>
            )}
          </div>
        )}

        {view.kind === 'installed' && (
          <div className="space-y-2 text-sm">
            <p>
              Installed <span className="font-medium">{view.installed.name}</span>: {view.installed.fileCount} file
              {view.installed.fileCount === 1 ? '' : 's'}.
            </p>
            <p className="text-muted-foreground">
              {view.installed.addedToInstance
                ? 'It has been added to this instance.'
                : 'It is in your library. Add it to an instance from that instance’s Content tab.'}
            </p>
            {view.installed.notes.length > 0 && (
              <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
                {view.installed.notes.map((note) => <li key={note}>{note}</li>)}
              </ul>
            )}
          </div>
        )}

        <DialogFooter className="gap-2 sm:justify-between">
          {view.kind === 'options' && (
            <>
              <div className="flex gap-2">
                <Button variant="outline" onClick={() => void load(true)} disabled={busy}>
                  Use defaults
                </Button>
              </div>
              <div className="flex gap-2">
                {page > 0 && (
                  <Button variant="outline" onClick={() => setPage(page - 1)} disabled={busy}>
                    Back
                  </Button>
                )}
                {page < lastPage ? (
                  <Button onClick={() => setPage(page + 1)} disabled={busy}>Next</Button>
                ) : (
                  <Button onClick={() => void review()} disabled={busy}>Review files</Button>
                )}
              </div>
            </>
          )}
          {view.kind === 'review' && (
            <>
              <Button variant="outline" onClick={() => setView({ kind: 'options' })} disabled={busy}>
                Back to options
              </Button>
              <Button onClick={() => void install()} disabled={busy || view.plan.files.length === 0}>
                Install these files
              </Button>
            </>
          )}
          {view.kind === 'error' && (
            <>
              <Button variant="outline" onClick={onClose}>Close</Button>
              {installer && (
                <Button onClick={() => setView({ kind: 'options' })}>Back to options</Button>
              )}
            </>
          )}
          {view.kind === 'installed' && <Button onClick={onClose}>Done</Button>}
          {view.kind === 'loading' && <Button variant="outline" onClick={onClose}>Cancel</Button>}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
