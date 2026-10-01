import { HideOnErrorImage } from '../components/HideOnErrorImage';
import { useEffect, useMemo, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import { defaultSchema, type Schema } from 'hast-util-sanitize';
import { ArrowLeft, ExternalLink, ShieldAlert, ShieldCheck } from 'lucide-react';
import {
  formatError,
  listContentProviders,
  listInstances,
  openExternalUrl,
  providerIdOf,
  providerInstallPack,
  providerInstallPreview,
  providerProject,
  providerVersions,
  type InstanceRow,
  type ProviderDescriptor,
  type ProviderPlanPreview,
  type ProviderSecurityNote,
  type ProviderProjectDetail,
  type ProviderProjectVersion,
} from '../lib/tauri';
import type { InstallIntent } from '../lib/installFlow';
import { InstallFlow } from '../components/InstallFlow';
import { showToast } from '../components/Toast';
import { useConfirm } from '@/components/ui/confirm';
import { choosePackInstanceName } from '@/lib/packInstanceName';

/**
 * Detail page for a project from any content provider that is not one of the
 * older, source-specific pages.
 *
 * Built only from the provider vocabulary core reports, so it works the same
 * for every provider plugin. It decides nothing: versions, install plans and
 * whether a plan is safe all come from core, and installing goes through the
 * same reviewed install flow as curated content.
 */

// Plugin-supplied text is Markdown only. Unlike the Modrinth body, raw HTML is
// not passed through at all (no rehype-raw), and what Markdown produces is
// still sanitised with https-only links and images.
const SANITIZE_SCHEMA: Schema = {
  ...defaultSchema,
  protocols: {
    ...defaultSchema.protocols,
    href: ['https'],
    src: ['https'],
  },
};

const https = (url: string | null | undefined) =>
  url && url.startsWith('https://') ? url : null;

const outsideFolders = (preview: ProviderPlanPreview | null) =>
  preview?.outsideContentFolders ?? [];

/** Whether this plan can be installed with the user's current settings. */
const blockedByLowSecurity = (preview: ProviderPlanPreview | null) =>
  !!preview &&
  ((preview.lowSecurity.length > 0 && !preview.lowSecurityAllowed) ||
    (outsideFolders(preview).length > 0 && !preview.reducedSecurityEnabled));

function NoteList({ notes }: { notes: ProviderSecurityNote[] }) {
  return (
    <ul className="mt-1 list-disc pl-6 text-xs text-muted-foreground">
      {notes.slice(0, 5).map((note, index) => (
        <li key={`${note.urlHost}-${index}`}>
          {note.urlHost || 'unknown host'}: {note.reason}
        </li>
      ))}
    </ul>
  );
}

/**
 * What the user is agreeing to, in the same tiers Technic has always used:
 * reduced assurance is a warning they can continue past; no integrity
 * information at all needs *Allow low security downloads*.
 */
function IntegrityNote({ preview }: { preview: ProviderPlanPreview }) {
  const hosts = Object.entries(preview.hosts);
  const outside = outsideFolders(preview);
  if (preview.warnings.length === 0 && preview.lowSecurity.length === 0 && outside.length === 0) {
    return (
      <p className="flex items-start gap-2 text-sm text-muted-foreground">
        <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-emerald-500" aria-hidden="true" />
        <span>
          Every file comes from a host {preview.providerTitle} declared, and Agora will check each
          one against the digest it published. That proves the files are the ones{' '}
          {preview.providerTitle} described — trusting {preview.providerTitle} is your call.
        </span>
      </p>
    );
  }
  return (
    <div className="space-y-2">
      {preview.lowSecurity.length > 0 && (
        <div className="rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm">
          <p className="flex items-start gap-2 font-medium">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-destructive" aria-hidden="true" />
            {preview.lowSecurity.length === 1 ? 'One file has' : `${preview.lowSecurity.length} files have`}{' '}
            no integrity information at all. Agora cannot detect a modified or swapped file.
          </p>
          <NoteList notes={preview.lowSecurity} />
          {!preview.lowSecurityAllowed && (
            <p className="mt-1 text-xs text-muted-foreground">
              Turn on Allow low security downloads in Settings → Content sources to install it.
            </p>
          )}
        </div>
      )}
      {outside.length > 0 && (
        <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
          <p className="flex items-start gap-2 font-medium">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-500" aria-hidden="true" />
            {outside.length === 1 ? 'One file goes' : `${outside.length} files go`} outside the usual
            content folders.
          </p>
          <ul className="mt-1 list-disc pl-6 text-xs text-muted-foreground">
            {outside.slice(0, 5).map((path) => (
              <li key={path}>{path}</li>
            ))}
          </ul>
          {!preview.reducedSecurityEnabled && (
            <p className="mt-1 text-xs text-muted-foreground">
              Turn on Reduced security mode in Settings → Content sources to install it.
            </p>
          )}
        </div>
      )}
      {preview.warnings.length > 0 && (
        <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
          <p className="flex items-start gap-2 font-medium">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-500" aria-hidden="true" />
            Reduced security: you can continue, but know what you are accepting.
          </p>
          <NoteList notes={preview.warnings} />
        </div>
      )}
      {hosts.length > 0 && (
        <p className="text-xs text-muted-foreground">
          Downloads from {hosts.map(([host, count]) => `${host} (${count})`).join(', ')}.
        </p>
      )}
    </div>
  );
}

/** Confirmation text for a plan with anything to warn about, or null. */
function warningBody(preview: ProviderPlanPreview | null): string | null {
  const outside = outsideFolders(preview);
  if (
    !preview ||
    (preview.warnings.length === 0 && preview.lowSecurity.length === 0 && outside.length === 0)
  ) {
    return null;
  }
  return [
    ...[...preview.lowSecurity, ...preview.warnings].map(
      (note) => `${note.urlHost || 'unknown host'}: ${note.reason}`,
    ),
    ...outside.map((path) => `${path}: outside the usual content folders`),
  ]
    .slice(0, 5)
    .join('\n');
}

export function ProviderDetail({
  itemId,
  initialInstanceId,
  onBack,
  onOpenInstanceEditor,
}: {
  itemId: string;
  initialInstanceId?: string;
  onBack: () => void;
  onOpenInstanceEditor?: (instanceId: string) => void;
}) {
  const { confirm, prompt } = useConfirm();
  const providerId = providerIdOf(itemId);
  const [descriptor, setDescriptor] = useState<ProviderDescriptor | null>(null);
  const [detail, setDetail] = useState<ProviderProjectDetail | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [instances, setInstances] = useState<InstanceRow[]>([]);
  const [instanceId, setInstanceId] = useState(initialInstanceId ?? '');
  const [versions, setVersions] = useState<ProviderProjectVersion[]>([]);
  const [versionsError, setVersionsError] = useState<string | null>(null);
  const [versionId, setVersionId] = useState('');
  const [preview, setPreview] = useState<ProviderPlanPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [installIntent, setInstallIntent] = useState<InstallIntent | null>(null);

  const isPack = detail?.project.contentType === 'pack';
  const instance = useMemo(
    () => instances.find((row) => row.instance_id === instanceId) ?? null,
    [instances, instanceId],
  );

  useEffect(() => {
    let cancelled = false;
    listContentProviders()
      .then((all) => {
        if (!cancelled && Array.isArray(all)) {
          setDescriptor(all.find((p) => p.id === providerId) ?? null);
        }
      })
      .catch(() => {});
    providerProject(itemId)
      .then((value) => {
        if (!cancelled) setDetail(value);
      })
      .catch((e) => {
        if (!cancelled) setDetailError(formatError(e));
      });
    listInstances()
      .then((rows) => {
        if (!cancelled && Array.isArray(rows)) setInstances(rows);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [itemId, providerId]);

  // Versions are listed for the chosen instance (or unfiltered for a pack,
  // which brings its own Minecraft version and loader).
  useEffect(() => {
    if (!detail) return;
    let cancelled = false;
    setVersionsError(null);
    providerVersions(
      itemId,
      isPack ? undefined : instance?.minecraft_version,
      isPack ? undefined : instance?.loader,
    )
      .then((response) => {
        if (cancelled) return;
        setVersions(response.versions);
        setVersionId((current) =>
          response.versions.some((v) => v.id === current) ? current : '',
        );
      })
      .catch((e) => {
        if (!cancelled) setVersionsError(formatError(e));
      });
    return () => {
      cancelled = true;
    };
  }, [detail, itemId, isPack, instance]);

  // What installing would involve, asked of core before anything downloads.
  useEffect(() => {
    if (!detail || (!isPack && !instance)) {
      setPreview(null);
      return;
    }
    let cancelled = false;
    providerInstallPreview(
      itemId,
      versionId || undefined,
      isPack ? undefined : instance?.minecraft_version,
      isPack ? undefined : instance?.loader,
    )
      .then((value) => {
        if (!cancelled) setPreview(value);
      })
      .catch(() => {
        if (!cancelled) setPreview(null);
      });
    return () => {
      cancelled = true;
    };
  }, [detail, itemId, isPack, instance, versionId]);

  const installIntoInstance = async () => {
    if (!instance) return;
    const warnings = warningBody(preview);
    if (warnings && !await confirm({
      title: 'Install with reduced security?',
      body: warnings,
      confirmLabel: 'Continue',
    })) return;
    setInstallIntent({
      action: {
        type: 'install',
        sourceType: 'provider',
        itemId,
        candidateVersion: versionId || undefined,
      },
      targetInstance: instance.instance_id,
      optionalDeps: { type: 'prompt' },
      requestedBy: 'interactive',
      overrides: {
        allowReplace: false,
        skipHealthScan: false,
        forceConflictResolution: {},
      },
    });
  };

  const installPack = async () => {
    if (!detail || busy) return;
    const title = descriptor?.title ?? 'this provider';
    const ok = await confirm({
      title: `Create a new instance from ${detail.project.title}?`,
      body: preview
        ? `${preview.fileCount} files from ${title}. ${
            warningBody(preview)
              ? `Reduced security:\n${warningBody(preview)}`
              : 'Each is checked against the digest the provider published.'
          }`
        : `Files come from ${title}.`,
      confirmLabel: 'Install pack',
    });
    if (!ok) return;
    setBusy(true);
    try {
      const instanceName = await choosePackInstanceName(prompt, detail.project.title);
      if (instanceName === null) return;
      const result = await providerInstallPack(itemId, versionId || undefined, instanceName);
      showToast(`Created ${result.name}.`, 'success');
      onOpenInstanceEditor?.(result.instance_id);
    } catch (e) {
      showToast(formatError(e), 'error');
    } finally {
      setBusy(false);
    }
  };

  const summary = detail?.project;
  const icon = https(summary?.iconUrl);

  return (
    <div className="space-y-6">
      <button
        type="button"
        onClick={onBack}
        className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-4 w-4" aria-hidden="true" /> Back
      </button>

      {detailError && (
        <div role="alert" className="rounded-lg border border-destructive bg-destructive/10 p-3 text-sm text-destructive">
          {detailError}
        </div>
      )}

      {summary && (
        <section className="agora-hero compact flex items-start gap-4">
          {icon ? (
            <img src={icon} alt="" className="h-16 w-16 rounded-lg object-cover" loading="lazy" />
          ) : null}
          <div className="min-w-0 space-y-1">
            <h2 className="text-2xl font-bold">{summary.title}</h2>
            <p className="text-sm text-muted-foreground">
              {summary.author ? `by ${summary.author} · ` : ''}
              from {descriptor?.title ?? providerId}
              {descriptor?.origin.kind === 'plugin' ? ' (plugin)' : ''}
            </p>
            {summary.description && <p className="text-sm">{summary.description}</p>}
            <div className="flex flex-wrap gap-2 pt-1 text-xs text-muted-foreground">
              {summary.downloads != null && <span>{summary.downloads.toLocaleString()} downloads</span>}
              {detail?.license && <span>License: {detail.license}</span>}
              {summary.categories.slice(0, 6).map((category) => (
                <span key={category} className="rounded bg-muted px-1.5 py-0.5">{category}</span>
              ))}
            </div>
          </div>
        </section>
      )}

      {detail && (
        <section className="space-y-3 rounded-lg border border-border p-4">
          <h3 className="font-semibold">Install</h3>
          {!isPack && (
            <label className="block text-sm">
              <span className="mb-1 block text-muted-foreground">Instance</span>
              <select
                value={instanceId}
                onChange={(event) => setInstanceId(event.target.value)}
                className="w-full rounded-lg border border-input bg-background px-3 py-2"
              >
                <option value="">Choose an instance…</option>
                {instances.map((row) => (
                  <option key={row.instance_id} value={row.instance_id}>
                    {row.name} ({row.minecraft_version} · {row.loader})
                  </option>
                ))}
              </select>
            </label>
          )}
          <label className="block text-sm">
            <span className="mb-1 block text-muted-foreground">Version</span>
            <select
              value={versionId}
              onChange={(event) => setVersionId(event.target.value)}
              className="w-full rounded-lg border border-input bg-background px-3 py-2"
              disabled={versions.length === 0}
            >
              <option value="">Newest compatible</option>
              {versions.map((version) => (
                <option key={version.id} value={version.id}>
                  {version.name}
                  {version.channel !== 'release' ? ` (${version.channel})` : ''}
                </option>
              ))}
            </select>
          </label>
          {versionsError && <p className="text-xs text-destructive">{versionsError}</p>}
          {!isPack && instance && versions.length === 0 && !versionsError && (
            <p className="text-xs text-muted-foreground">
              No versions for Minecraft {instance.minecraft_version} with {instance.loader}.
            </p>
          )}
          {preview && <IntegrityNote preview={preview} />}
          <div className="flex gap-2">
            {isPack ? (
              <button
                type="button"
                onClick={installPack}
                disabled={busy || blockedByLowSecurity(preview)}
                className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              >
                {busy ? 'Installing…' : 'Install as new instance'}
              </button>
            ) : (
              <button
                type="button"
                onClick={installIntoInstance}
                disabled={!instance || instance.is_locked || blockedByLowSecurity(preview)}
                title={instance?.is_locked ? 'Unlock the instance to install content.' : undefined}
                className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              >
                Review install
              </button>
            )}
          </div>
        </section>
      )}

      {detail?.body && (
        <section className="prose prose-sm dark:prose-invert max-w-none text-foreground">
          <ReactMarkdown
            rehypePlugins={[[rehypeSanitize, SANITIZE_SCHEMA]]}
            components={{
              a: ({ node: _node, ...props }) => (
                <a {...props} target="_blank" rel="noopener noreferrer" />
              ),
              img: ({ node: _node, ...props }) => (
                <HideOnErrorImage {...props} />
              ),
            }}
          >
            {detail.body}
          </ReactMarkdown>
        </section>
      )}

      {detail && detail.links.length > 0 && (
        <section className="flex flex-wrap gap-2">
          {detail.links
            .filter((link) => https(link.url))
            .map((link) => (
              <button
                key={link.url}
                type="button"
                onClick={() => openExternalUrl(link.url).catch((e) => showToast(formatError(e), 'error'))}
                className="inline-flex items-center gap-1 rounded-lg border border-input px-3 py-1.5 text-sm hover:bg-accent"
              >
                {link.label} <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
              </button>
            ))}
        </section>
      )}

      {installIntent && instance && (
        <InstallFlow
          open
          intent={installIntent}
          instanceName={instance.name}
          onClose={() => setInstallIntent(null)}
          onOpenInstance={onOpenInstanceEditor}
        />
      )}
    </div>
  );
}
