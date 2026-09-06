import { useCallback, useEffect, useState } from 'react';
import { FileStack, Star, Trash2 } from 'lucide-react';
import {
  deleteInstanceTemplate,
  formatError,
  getSetting,
  listInstanceTemplates,
  setSetting,
  updateInstanceTemplate,
  type InstanceTemplate,
} from '../../lib/tauri';
import { SettingsSection } from './SettingsSection';

const DEFAULT_TEMPLATE_SETTING = 'default_instance_template';

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Human summary of what a template will actually do when applied. */
function describeTemplate(template: InstanceTemplate): string {
  const parts: string[] = [];
  if (template.files.length > 0) {
    const bytes = template.files.reduce((sum, file) => sum + file.size, 0);
    parts.push(
      `${template.files.length} config file${template.files.length === 1 ? '' : 's'} (${formatBytes(bytes)})`,
    );
  }
  const jvm = template.jvm ?? {};
  const jvmKeys = (Object.keys(jvm) as (keyof typeof jvm)[]).filter(
    (key) => jvm[key] !== null && jvm[key] !== undefined,
  );
  if (jvmKeys.length > 0) {
    parts.push(`${jvmKeys.length} Java setting${jvmKeys.length === 1 ? '' : 's'}`);
  }
  // A template with neither half is legal (the user cleared it) and saying so
  // is more useful than rendering an empty line.
  return parts.length > 0 ? parts.join(' · ') : 'Empty — applies nothing';
}

/**
 * Manage saved instance templates.
 *
 * Templates are *captured* from an instance (Instance editor → Templates), so
 * this page deliberately offers no "new template" button — only rename, set
 * default, and delete.
 */
export function TemplateSettings() {
  const [templates, setTemplates] = useState<InstanceTemplate[]>([]);
  const [defaultId, setDefaultId] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draftName, setDraftName] = useState('');

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const [available, stored] = await Promise.all([
        listInstanceTemplates(),
        getSetting(DEFAULT_TEMPLATE_SETTING),
      ]);
      setTemplates(available);
      setDefaultId(typeof stored === 'string' ? stored : '');
      setError(null);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void reload(); }, [reload]);

  const makeDefault = async (id: string) => {
    // Clicking the current default clears it, which is the only way to get back
    // to "no template" without deleting the template itself.
    const next = defaultId === id ? '' : id;
    try {
      await setSetting(DEFAULT_TEMPLATE_SETTING, next);
      setDefaultId(next);
    } catch (e) {
      setError(formatError(e));
    }
  };

  const commitRename = async (id: string) => {
    const name = draftName.trim();
    setEditingId(null);
    if (!name) return;
    try {
      await updateInstanceTemplate({ templateId: id, name });
      await reload();
    } catch (e) {
      setError(formatError(e));
    }
  };

  const remove = async (template: InstanceTemplate) => {
    try {
      await deleteInstanceTemplate(template.id);
      await reload();
    } catch (e) {
      setError(formatError(e));
    }
  };

  return (
    <SettingsSection
      icon={FileStack}
      title="Instance templates"
      description="Saved config files and Java settings that new instances can start from. Capture one from an instance's Templates tab."
      data-testid="template-settings"
    >
      {error && <p className="text-sm text-destructive">{error}</p>}
      {loading && <p className="text-sm text-muted-foreground">Loading templates…</p>}

      {!loading && templates.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No templates yet. Open an instance, go to its Templates tab, and save one.
        </p>
      )}

      {templates.map((template) => (
        <div
          key={template.id}
          className="flex items-start gap-3 rounded-lg border border-border bg-background p-3"
        >
          <div className="min-w-0 flex-1">
            {editingId === template.id ? (
              <input
                autoFocus
                value={draftName}
                aria-label="Template name"
                onChange={(e) => setDraftName(e.target.value)}
                onBlur={() => void commitRename(template.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void commitRename(template.id);
                  if (e.key === 'Escape') setEditingId(null);
                }}
                className="w-full rounded-md border border-input bg-background px-2 py-1 text-sm"
              />
            ) : (
              <button
                type="button"
                onClick={() => { setEditingId(template.id); setDraftName(template.name); }}
                className="truncate text-sm font-medium hover:underline"
              >
                {template.name}
              </button>
            )}
            {template.description && (
              <p className="mt-0.5 truncate text-xs text-muted-foreground">{template.description}</p>
            )}
            <p className="mt-0.5 text-xs text-muted-foreground">{describeTemplate(template)}</p>
          </div>

          <button
            type="button"
            onClick={() => void makeDefault(template.id)}
            aria-pressed={defaultId === template.id}
            title={defaultId === template.id ? 'Stop using as default' : 'Use as default for new instances'}
            className="rounded-md border border-input p-1.5 hover:bg-accent"
          >
            <Star
              className={`h-4 w-4 ${defaultId === template.id ? 'fill-current text-primary' : 'text-muted-foreground'}`}
            />
            <span className="sr-only">
              {defaultId === template.id ? 'Stop using as default' : 'Use as default'}
            </span>
          </button>

          <button
            type="button"
            onClick={() => void remove(template)}
            title="Delete template"
            className="rounded-md border border-input p-1.5 text-destructive hover:bg-accent"
          >
            <Trash2 className="h-4 w-4" />
            <span className="sr-only">Delete {template.name}</span>
          </button>
        </div>
      ))}
    </SettingsSection>
  );
}
