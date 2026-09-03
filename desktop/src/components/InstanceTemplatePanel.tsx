import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  applyInstanceTemplate,
  createInstanceTemplate,
  formatError,
  listCapturableTemplateFiles,
  listInstanceTemplates,
  type CapturableFile,
  type InstanceRow,
  type InstanceTemplate,
  type TemplateJvm,
} from '../lib/tauri';

const CATEGORY_LABELS: Record<string, string> = {
  options: 'Game options',
  servers: 'Server list',
  config: 'Mod configs (config/)',
  defaultconfigs: 'Default configs (defaultconfigs/)',
  kubejs: 'KubeJS scripts (kubejs/)',
};

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** JVM settings of the current instance, in template shape. */
function jvmFromRow(row: InstanceRow | undefined): TemplateJvm {
  if (!row) return {};
  return {
    java_path: row.java_path ?? null,
    jvm_memory_mb: row.jvm_memory_mb ?? null,
    jvm_memory_mode: row.jvm_memory_mode ?? null,
    jvm_gc: row.jvm_gc ?? null,
    jvm_custom_args: row.jvm_custom_args || null,
    jvm_always_pre_touch: row.jvm_always_pre_touch ?? null,
  };
}

/**
 * Capture this instance's configuration into a reusable template, and apply an
 * existing template back onto it.
 *
 * The file picker starts with nothing selected on purpose: a template is meant
 * to be a deliberate, small set of configs the user actually wants to carry
 * forward, not a mirror of the whole instance.
 */
export function InstanceTemplatePanel({
  instanceId,
  row,
  disabled = false,
  onApplied,
}: {
  instanceId: string;
  row?: InstanceRow;
  disabled?: boolean;
  onApplied?: () => void;
}) {
  const [capturable, setCapturable] = useState<CapturableFile[]>([]);
  const [selected, setSelected] = useState<Record<string, boolean>>({});
  const [templates, setTemplates] = useState<InstanceTemplate[]>([]);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [includeJvm, setIncludeJvm] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const [files, stored] = await Promise.all([
        listCapturableTemplateFiles(instanceId),
        listInstanceTemplates(),
      ]);
      setCapturable(files);
      setTemplates(stored);
      setError(null);
    } catch (e) {
      setError(formatError(e));
    }
  }, [instanceId]);

  useEffect(() => { void reload(); }, [reload]);

  const grouped = useMemo(() => {
    const buckets = new Map<string, CapturableFile[]>();
    for (const file of capturable) {
      const bucket = buckets.get(file.category);
      if (bucket) bucket.push(file);
      else buckets.set(file.category, [file]);
    }
    return [...buckets.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [capturable]);

  const selectedPaths = useMemo(
    () => capturable.filter((file) => selected[file.relative_path] && !file.too_large)
      .map((file) => file.relative_path),
    [capturable, selected],
  );

  const save = async () => {
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      const created = await createInstanceTemplate({
        name: name.trim() || `${row?.name ?? instanceId} template`,
        description: description.trim() || null,
        jvm: includeJvm ? jvmFromRow(row) : null,
        sourceInstanceId: selectedPaths.length > 0 ? instanceId : null,
        selectedPaths,
      });
      setName('');
      setDescription('');
      setSelected({});
      setStatus(`Saved template "${created.name}".`);
      await reload();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const apply = async (template: InstanceTemplate) => {
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      const outcome = await applyInstanceTemplate(instanceId, template.id);
      // A file count of zero has three different meanings, so report what
      // actually happened rather than inferring from the number alone.
      const parts: string[] = [];
      if (outcome.files_applied > 0) {
        parts.push(`${outcome.files_applied} file${outcome.files_applied === 1 ? '' : 's'}`);
      }
      if (outcome.jvm_applied) parts.push('Java settings');
      const damaged = outcome.files_missing > 0
        ? ` ${outcome.files_missing} file${outcome.files_missing === 1 ? ' was' : 's were'} listed but missing from the template.`
        : '';
      const undo = outcome.undo_snapshot_id
        ? ' The previous state is saved as a snapshot you can restore.'
        : '';
      setStatus(
        parts.length === 0
          ? `"${template.name}" had nothing to apply.${damaged}`
          : `Applied ${parts.join(' and ')} from "${template.name}".${damaged}${undo}`,
      );
      onApplied?.();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="rounded-xl border border-border bg-card p-4 space-y-4" data-testid="instance-templates">
      <div>
        <h3 className="font-semibold text-sm">Save as template</h3>
        <p className="mt-1 text-xs text-muted-foreground">
          Pick the config files worth reusing. New instances can start from this template,
          and you can set a default in Settings → General → Templates.
        </p>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <label className="block">
          <span className="text-sm font-medium">Template name</span>
          <input
            value={name}
            disabled={disabled || busy}
            onChange={(e) => setName(e.target.value)}
            placeholder={`${row?.name ?? instanceId} template`}
            className="mt-1 w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
          />
        </label>
        <label className="block">
          <span className="text-sm font-medium">Description (optional)</span>
          <input
            value={description}
            disabled={disabled || busy}
            onChange={(e) => setDescription(e.target.value)}
            className="mt-1 w-full rounded-lg border border-input bg-background px-3 py-2 text-sm"
          />
        </label>
      </div>

      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={includeJvm}
          disabled={disabled || busy}
          onChange={(e) => setIncludeJvm(e.target.checked)}
        />
        Include this instance&apos;s Java and memory settings
      </label>

      {capturable.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          This instance has no config files to capture yet.
        </p>
      ) : (
        <div className="max-h-72 space-y-3 overflow-y-auto rounded-lg border border-border p-3">
          {grouped.map(([category, files]) => {
            const selectable = files.filter((file) => !file.too_large);
            const allSelected = selectable.length > 0
              && selectable.every((file) => selected[file.relative_path]);
            return (
              <div key={category}>
                <div className="flex items-center justify-between">
                  <span className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                    {CATEGORY_LABELS[category] ?? category}
                  </span>
                  <button
                    type="button"
                    disabled={disabled || busy || selectable.length === 0}
                    onClick={() => setSelected((prev) => {
                      const next = { ...prev };
                      for (const file of selectable) next[file.relative_path] = !allSelected;
                      return next;
                    })}
                    className="text-xs text-primary hover:underline disabled:opacity-50"
                  >
                    {allSelected ? 'Clear' : 'Select all'}
                  </button>
                </div>
                {files.map((file) => (
                  <label
                    key={file.relative_path}
                    className="mt-1 flex items-center gap-2 text-sm"
                    title={file.too_large ? 'Too large to include in a template' : undefined}
                  >
                    <input
                      type="checkbox"
                      checked={Boolean(selected[file.relative_path])}
                      disabled={disabled || busy || file.too_large}
                      onChange={(e) => setSelected((prev) => ({
                        ...prev,
                        [file.relative_path]: e.target.checked,
                      }))}
                    />
                    <span className="truncate">{file.relative_path}</span>
                    <span className="ml-auto shrink-0 text-xs text-muted-foreground">
                      {file.too_large ? 'too large' : formatBytes(file.size)}
                    </span>
                  </label>
                ))}
              </div>
            );
          })}
        </div>
      )}

      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={() => void save()}
          disabled={disabled || busy}
          className="rounded-lg bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
        >
          {busy ? 'Saving…' : 'Save template'}
        </button>
        <span className="text-xs text-muted-foreground">
          {selectedPaths.length} file{selectedPaths.length === 1 ? '' : 's'} selected
        </span>
      </div>

      {templates.length > 0 && (
        <div className="space-y-2 border-t border-border pt-4">
          <h3 className="font-semibold text-sm">Apply a template</h3>
          <p className="text-xs text-muted-foreground">
            Overwrites this instance&apos;s copies of the files the template holds.
          </p>
          {templates.map((template) => (
            <div
              key={template.id}
              className="flex items-center gap-3 rounded-lg border border-border bg-background p-2"
            >
              <span className="min-w-0 flex-1 truncate text-sm">{template.name}</span>
              <span className="shrink-0 text-xs text-muted-foreground">
                {template.files.length} file{template.files.length === 1 ? '' : 's'}
              </span>
              <button
                type="button"
                onClick={() => void apply(template)}
                disabled={disabled || busy}
                className="rounded-lg border border-input px-3 py-1 text-sm hover:bg-accent disabled:opacity-50"
              >
                Apply
              </button>
            </div>
          ))}
        </div>
      )}

      {status && <p className="text-sm text-muted-foreground">{status}</p>}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </section>
  );
}
