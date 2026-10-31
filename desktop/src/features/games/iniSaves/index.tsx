/**
 * The "INI & saves" tab of a non-Minecraft game's instance (MASTER_SPEC §26.7): the game's own
 * configuration files, edited per instance, and the folder the game keeps its saves in.
 *
 * Each section loads and fails on its own, so a problem with one never hides the other.
 */

import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useConfirm } from '@/components/ui/confirm';
import {
  describeFailure,
  getIni,
  getSaves,
  setIniValue,
  setSaves,
  type IniChange,
  type IniContents,
  type IniEntryRow,
  type IniFileRow,
  type IniSourceName,
  type SaveChoice,
  type SaveFolderRow,
  type SavesChange,
} from './api';

interface InstanceIniSavesTabProps {
  gameId: string;
  instanceId: string;
}

export function InstanceIniSavesTab({ instanceId }: InstanceIniSavesTabProps) {
  return (
    <section className="space-y-8" aria-label="INI and saves" data-testid="ini-saves-tab">
      <SavesSection instanceId={instanceId} />
      <IniSection instanceId={instanceId} />
    </section>
  );
}

type ReadState<T> = { status: 'loading' } | { status: 'failed'; message: string } | { status: 'loaded'; value: T };

/**
 * Runs `read` for the instance on mount and again when the instance changes. A reply for an older
 * request is dropped, so a slow answer for the last instance never replaces the current one.
 */
function useReadOnMount<T>(read: (instanceId: string) => Promise<T>, instanceId: string) {
  const [state, setState] = useState<ReadState<T>>({ status: 'loading' });
  const requestId = useRef(0);

  const refetch = useCallback(() => {
    const id = ++requestId.current;
    setState({ status: 'loading' });
    read(instanceId)
      .then((value) => {
        if (id === requestId.current) setState({ status: 'loaded', value });
      })
      .catch((failure: unknown) => {
        if (id === requestId.current) setState({ status: 'failed', message: describeFailure(failure) });
      });
  }, [read, instanceId]);

  useEffect(() => {
    refetch();
    return () => {
      requestId.current++;
    };
  }, [refetch]);

  return { state, refetch, setState };
}

function SectionFrame({ title, id, children }: { title: string; id: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="space-y-4">
      <h2 id={id} className="text-lg font-semibold">
        {title}
      </h2>
      {children}
    </section>
  );
}

function FailureNote({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="space-y-3">
      <p role="alert" className="rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
        {message}
      </p>
      <Button variant="outline" size="sm" onClick={onRetry}>
        Try again
      </Button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Save folder
// ---------------------------------------------------------------------------

const SAVE_CHOICE_WORDS: Record<SaveChoice, { label: string; plain: string }> = {
  shared: {
    label: 'Shared saves',
    plain: "The game's own shared save folder, which every instance that uses shared saves reads.",
  },
  own: {
    label: 'Own saves',
    plain: "A save folder of this instance's own, so its saves stay apart from the shared ones.",
  },
};

function folderLine(folder: SaveFolderRow): string {
  if (!folder.exists) return 'Not made yet. The game creates it.';
  if (folder.saves === 0) return 'No saves yet.';
  const count = `${folder.saves} ${folder.saves === 1 ? 'save' : 'saves'}`;
  return folder.newest ? `${count}, newest ${folder.newest}` : count;
}

function settingSourceWords(source: IniSourceName): string {
  if (source === 'copy') return "the instance's copy";
  if (source === 'game_file') return "the game's file";
  return 'no file yet';
}

/** What a change of the save choice did to the game's setting, in plain words. */
function settingNotice(change: SavesChange): string {
  const { action, value } = change.setting;
  switch (action) {
    case 'set':
      return `The game's save setting now names ${value ?? 'that folder'}.`;
    case 'unchanged':
      return "The game's save setting already named that folder.";
    case 'restored':
      return `The game's save setting is back to ${value ?? 'its old value'}, as it was before.`;
    case 'removed':
      return "The game's save setting was removed; it was not set before.";
    case 'left_alone':
      return `The game's save setting no longer holds the value Agora set (it holds ${value ?? 'nothing'}), so it was left as it is. Check it by hand.`;
    case 'nothing_recorded':
      return "Nothing was recorded for this instance, so the game's save setting was left as it is.";
  }
}

function SavesSection({ instanceId }: { instanceId: string }) {
  const { state, refetch, setState } = useReadOnMount(getSaves, instanceId);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const choose = async (choice: SaveChoice) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const change = await setSaves(instanceId, choice);
      setState({ status: 'loaded', value: change.saves });
      const moved = change.previous === change.saves.choice
        ? `This instance already keeps ${SAVE_CHOICE_WORDS[choice].label.toLowerCase()}.`
        : `This instance now keeps ${SAVE_CHOICE_WORDS[choice].label.toLowerCase()}.`;
      setNotice(`${moved} ${settingNotice(change)} Your saves were not moved, copied or deleted.`);
    } catch (failure: unknown) {
      setError(describeFailure(failure));
    } finally {
      setBusy(false);
    }
  };

  return (
    <SectionFrame title="Save folder" id="ini-saves-folder-heading">
      {state.status === 'loading' && <p className="text-sm text-muted-foreground">Reading the save choice…</p>}
      {state.status === 'failed' && <FailureNote message={state.message} onRetry={refetch} />}
      {state.status === 'loaded' && (
        <div className="space-y-4" data-testid="saves-panel">
          <p className="max-w-2xl text-sm text-muted-foreground">
            The game keeps its saves in one of two folders. Switching changes which one this instance uses. It never
            moves a save: the saves in the other folder stay where they are.
          </p>
          <div className="grid gap-3 md:grid-cols-2">
            {(['shared', 'own'] as const).map((choice) => {
              const current = state.value.choice === choice;
              return (
                <div
                  key={choice}
                  className={[
                    'space-y-2 rounded-xl border p-4',
                    current ? 'border-primary bg-primary/5' : 'border-border bg-card',
                  ].join(' ')}
                >
                  <div className="flex items-center justify-between gap-2">
                    <h3 className="font-semibold">{SAVE_CHOICE_WORDS[choice].label}</h3>
                    {current && <span className="text-xs font-medium text-primary">In use</span>}
                  </div>
                  <p className="text-sm text-muted-foreground">{SAVE_CHOICE_WORDS[choice].plain}</p>
                  {!current && (
                    <Button variant="outline" size="sm" onClick={() => choose(choice)} disabled={busy}>
                      Use {SAVE_CHOICE_WORDS[choice].label.toLowerCase()}
                    </Button>
                  )}
                </div>
              );
            })}
          </div>
          <dl className="grid gap-2 text-sm md:grid-cols-[max-content_1fr]">
            <dt className="text-muted-foreground">In use</dt>
            <dd className="break-all font-mono text-xs">
              {state.value.inUse.path} <span className="font-sans">({folderLine(state.value.inUse)})</span>
            </dd>
            <dt className="text-muted-foreground">Other folder</dt>
            <dd className="break-all font-mono text-xs">
              {state.value.other.path} <span className="font-sans">({folderLine(state.value.other)})</span>
            </dd>
            <dt className="text-muted-foreground">Game setting</dt>
            <dd>
              {state.value.setting} in {state.value.settingFile} is{' '}
              <span className="font-mono text-xs">{state.value.settingValue ?? 'not set'}</span> (from{' '}
              {settingSourceWords(state.value.settingSource)}).
            </dd>
          </dl>
        </div>
      )}
      {error && (
        <p role="alert" className="rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
          {error}
        </p>
      )}
      {notice && (
        <p role="status" className="text-sm text-muted-foreground">
          {notice}
        </p>
      )}
    </SectionFrame>
  );
}

// ---------------------------------------------------------------------------
// INI files
// ---------------------------------------------------------------------------

function fileStatus(file: IniFileRow): string {
  if (file.copyExists) return "The instance's copy";
  if (file.gameFileExists) return "The game's file";
  return 'Not there yet';
}

/** The sections of a file, in the order they first appear, with their keys. */
function groupBySection(entries: IniEntryRow[]): Array<{ section: string; entries: IniEntryRow[] }> {
  const groups: Array<{ section: string; entries: IniEntryRow[] }> = [];
  for (const entry of entries) {
    const group = groups.find((candidate) => candidate.section === entry.section);
    if (group) group.entries.push(entry);
    else groups.push({ section: entry.section, entries: [entry] });
  }
  return groups;
}

function IniSection({ instanceId }: { instanceId: string }) {
  const { confirm } = useConfirm();
  const { state, refetch, setState } = useReadOnMount(getIni, instanceId);
  const [selected, setSelected] = useState<string | null>(null);
  const [contents, setContents] = useState<IniContents | null>(null);
  const [fileState, setFileState] = useState<'idle' | 'loading' | 'failed'>('idle');
  const [fileError, setFileError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [newSection, setNewSection] = useState('');
  const [newKey, setNewKey] = useState('');
  const [newValue, setNewValue] = useState('');

  // Pick the first file once the list arrives, and keep a file the user picked while it still exists.
  const files = state.status === 'loaded' ? state.value.files : [];
  const activeFile = selected !== null && files.some((file) => file.instancePath === selected)
    ? selected
    : files[0]?.instancePath ?? null;

  useEffect(() => {
    if (activeFile === null) {
      setContents(null);
      return;
    }
    let cancelled = false;
    setFileState('loading');
    setFileError(null);
    getIni(instanceId, activeFile)
      .then((view) => {
        if (cancelled) return;
        setContents(view.contents);
        setFileState('idle');
      })
      .catch((failure: unknown) => {
        if (cancelled) return;
        setFileError(describeFailure(failure));
        setFileState('failed');
      });
    return () => {
      cancelled = true;
    };
  }, [instanceId, activeFile]);

  /** Applies the reply to a key change: the file list and the file shown both come from it. */
  const applyChange = (change: IniChange) => {
    setState({ status: 'loaded', value: change.view });
    if (change.view.contents) setContents(change.view.contents);
  };

  const saveKey = async (file: string, section: string, key: string, value: string | null) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const change = await setIniValue(instanceId, file, section, key, value);
      applyChange(change);
      if (value === null) {
        setNotice(change.changed ? `Removed ${key} from [${section}].` : `${key} was not set in [${section}]; nothing changed.`);
      } else {
        setNotice(change.changed ? `Saved ${key} in [${section}].` : `${key} in [${section}] already has that value.`);
      }
      return true;
    } catch (failure: unknown) {
      setError(describeFailure(failure));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const removeKey = async (file: string, entry: IniEntryRow) => {
    const confirmed = await confirm({
      title: `Remove ${entry.key} from [${entry.section}]?`,
      body: `Agora takes this key out of ${file} for this instance only. The game then uses its own default for it.`,
      confirmLabel: 'Remove key',
      tone: 'danger',
    });
    if (!confirmed) return;
    await saveKey(file, entry.section, entry.key, null);
  };

  const addKey = async () => {
    if (activeFile === null) return;
    const saved = await saveKey(activeFile, newSection.trim(), newKey.trim(), newValue);
    if (saved) {
      setNewKey('');
      setNewValue('');
    }
  };

  return (
    <SectionFrame title="Game settings (INI files)" id="ini-saves-ini-heading">
      {state.status === 'loading' && <p className="text-sm text-muted-foreground">Reading the INI files…</p>}
      {state.status === 'failed' && <FailureNote message={state.message} onRetry={refetch} />}
      {state.status === 'loaded' && files.length === 0 && (
        <p className="rounded-xl border border-dashed border-border bg-card/60 p-4 text-sm text-muted-foreground">
          This game keeps no per-user INI files for this instance's store, so there is nothing to edit here.
        </p>
      )}
      {state.status === 'loaded' && files.length > 0 && (
        <div className="space-y-4" data-testid="ini-panel">
          <p className="max-w-2xl text-sm text-muted-foreground">
            These are the game's settings files. Changing a key here changes this instance only. The first change makes
            the instance's own copy; the game's file is not changed.
          </p>
          <div className="flex flex-wrap gap-2" role="group" aria-label="INI files">
            {files.map((file) => {
              const isActive = file.instancePath === activeFile;
              return (
                <button
                  key={file.instancePath}
                  type="button"
                  aria-pressed={isActive}
                  onClick={() => setSelected(file.instancePath)}
                  className={[
                    'rounded-lg px-3 py-1.5 text-left text-sm transition-colors',
                    isActive
                      ? 'bg-primary text-primary-foreground shadow-sm'
                      : 'border border-border text-muted-foreground hover:bg-accent hover:text-accent-foreground',
                  ].join(' ')}
                >
                  <span className="font-mono">{file.instancePath}</span>{' '}
                  <span className="text-xs opacity-80">({fileStatus(file)})</span>
                </button>
              );
            })}
          </div>

          {fileState === 'loading' && <p className="text-sm text-muted-foreground">Reading the file…</p>}
          {fileState === 'failed' && fileError && (
            <p role="alert" className="rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
              {fileError}
            </p>
          )}
          {fileState === 'idle' && contents && (
            <div className="space-y-3">
              {contents.source === 'game_file' && (
                <p className="text-sm text-muted-foreground">
                  Showing the game's own file, as the instance has no copy yet. Saving a key makes the instance's copy from
                  it.
                </p>
              )}
              {contents.source === 'nothing' && (
                <p className="text-sm text-muted-foreground">This file does not exist yet. Saving a key creates it.</p>
              )}
              {contents.entries.length === 0 && contents.source !== 'nothing' && (
                <p className="text-sm text-muted-foreground">This file has no keys yet.</p>
              )}
              {groupBySection(contents.entries).map((group) => (
                <div key={group.section} className="space-y-2">
                  <h3 className="font-mono text-sm font-semibold">[{group.section}]</h3>
                  <ul className="divide-y divide-border rounded-xl border border-border">
                    {group.entries.map((entry, index) => (
                      <IniKeyRow
                        key={`${entry.key}-${index}`}
                        entry={entry}
                        disabled={busy}
                        onSave={(value) => saveKey(contents.instancePath, entry.section, entry.key, value)}
                        onRemove={() => removeKey(contents.instancePath, entry)}
                      />
                    ))}
                  </ul>
                </div>
              ))}
            </div>
          )}

          <form
            className="grid gap-2 rounded-xl border border-border bg-card p-4 md:grid-cols-[1fr_1fr_1fr_auto] md:items-end"
            onSubmit={(event) => {
              event.preventDefault();
              if (newSection.trim() && newKey.trim()) void addKey();
            }}
          >
            <label className="space-y-1 text-sm">
              <span className="text-muted-foreground">Section</span>
              <Input value={newSection} onChange={(event) => setNewSection(event.target.value)} placeholder="General" />
            </label>
            <label className="space-y-1 text-sm">
              <span className="text-muted-foreground">Key</span>
              <Input value={newKey} onChange={(event) => setNewKey(event.target.value)} placeholder="sLanguage" />
            </label>
            <label className="space-y-1 text-sm">
              <span className="text-muted-foreground">Value</span>
              <Input value={newValue} onChange={(event) => setNewValue(event.target.value)} />
            </label>
            <Button type="submit" disabled={busy || !newSection.trim() || !newKey.trim() || activeFile === null}>
              Save key
            </Button>
          </form>
        </div>
      )}
      {error && (
        <p role="alert" className="rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
          {error}
        </p>
      )}
      {notice && (
        <p role="status" className="text-sm text-muted-foreground">
          {notice}
        </p>
      )}
    </SectionFrame>
  );
}

interface IniKeyRowProps {
  entry: IniEntryRow;
  disabled: boolean;
  onSave: (value: string) => Promise<boolean>;
  onRemove: () => void;
}

function IniKeyRow({ entry, disabled, onSave, onRemove }: IniKeyRowProps) {
  const [draft, setDraft] = useState(entry.value);

  // A change that came back from the backend replaces what the field showed.
  useEffect(() => {
    setDraft(entry.value);
  }, [entry.value]);

  return (
    <li className="flex flex-wrap items-center gap-2 px-3 py-2">
      <span className="min-w-40 font-mono text-sm">{entry.key}</span>
      <Input
        aria-label={`Value of ${entry.key} in [${entry.section}]`}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        className="h-9 min-w-48 flex-1 font-mono text-sm"
        disabled={disabled}
      />
      <Button size="sm" onClick={() => void onSave(draft)} disabled={disabled || draft === entry.value}>
        Save
      </Button>
      <Button size="sm" variant="outline" onClick={onRemove} disabled={disabled}>
        Remove
      </Button>
    </li>
  );
}
