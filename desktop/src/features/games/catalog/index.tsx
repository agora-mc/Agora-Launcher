/**
 * Catalog slot owned by the catalog slice: the Library tab's curated catalog section.
 *
 * The curated items for this game. Preview shows the file Agora would fetch and the checks it
 * would make; Install fetches, checks, stores and places it in the chosen instance. An archive
 * with an installer opens the FOMOD wizard (the content slice's component, imported from its
 * folder).
 */

import { useEffect, useState } from 'react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { formatBytes } from '@/components/installed-content/contentTableState';
import { formatError } from '@/lib/tauri';
import { FomodWizard } from '../fomod';
import {
  installCatalogEntry,
  listCatalogEntries,
  type CatalogEntry,
  type CatalogInstallReply,
  type CatalogListing,
} from './api';
import { hashOutcome } from './hashOutcome';

type ListState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'loaded'; listing: CatalogListing };

type EntryState =
  | { status: 'busy'; dryRun: boolean }
  | { status: 'error'; message: string }
  | { status: 'done'; reply: CatalogInstallReply };

export function LibraryCatalogSection({ gameId }: { gameId: string }) {
  const [list, setList] = useState<ListState>({ status: 'loading' });
  const [instanceId, setInstanceId] = useState('');
  const [installAnyway, setInstallAnyway] = useState(false);
  const [results, setResults] = useState<Record<string, EntryState>>({});

  const load = () => {
    setList({ status: 'loading' });
    listCatalogEntries(gameId)
      .then((listing) => {
        setList({ status: 'loaded', listing });
        setInstanceId((current) => current || listing.instances[0]?.instanceId || '');
      })
      .catch((error: unknown) => setList({ status: 'error', message: formatError(error) }));
  };

  useEffect(() => {
    load();
    // Loaded again only when the game changes; "Try again" calls load().
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gameId]);

  const act = (entry: CatalogEntry, dryRun: boolean) => {
    setResults((current) => ({ ...current, [entry.id]: { status: 'busy', dryRun } }));
    installCatalogEntry({ gameId, instanceId, entryId: entry.id, installAnyway, dryRun })
      .then((reply) => setResults((current) => ({ ...current, [entry.id]: { status: 'done', reply } })))
      .catch((error: unknown) =>
        setResults((current) => ({ ...current, [entry.id]: { status: 'error', message: formatError(error) } })),
      );
  };

  return (
    <section className="space-y-4" aria-labelledby="catalog-heading" data-testid="catalog-section">
      <header className="space-y-1">
        <h2 id="catalog-heading" className="text-base font-semibold">
          Curated items
        </h2>
        <p className="text-sm text-muted-foreground">
          Items Agora has checked for this game. Agora downloads only the file, from where its entry says it comes from,
          checks it, and then puts it in the instance you choose.
        </p>
      </header>

      {list.status === 'loading' && (
        <p role="status" className="text-sm text-muted-foreground">
          Loading the curated items.
        </p>
      )}

      {list.status === 'error' && (
        <div role="alert" className="space-y-2 text-sm">
          <p className="text-destructive">Agora could not load the curated items: {list.message}</p>
          <Button variant="outline" size="sm" onClick={load}>
            Try again
          </Button>
        </div>
      )}

      {list.status === 'loaded' && (
        <>
          {list.listing.instances.length === 0 ? (
            <p className="text-sm text-muted-foreground" data-testid="catalog-no-instance">
              Create an instance on the Instances tab first. Items are installed into an instance.
            </p>
          ) : (
            <div className="flex flex-wrap items-center gap-4 text-sm">
              <label className="grid gap-1">
                <span className="font-medium">Install into</span>
                <select
                  className="rounded-lg border border-border bg-background px-3 py-2"
                  value={instanceId}
                  onChange={(event) => setInstanceId(event.target.value)}
                >
                  {list.listing.instances.map((instance) => (
                    <option key={instance.instanceId} value={instance.instanceId}>
                      {instance.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="flex items-start gap-2">
                <input
                  type="checkbox"
                  className="mt-1"
                  checked={installAnyway}
                  onChange={(event) => setInstallAnyway(event.target.checked)}
                />
                <span>
                  Install even if the file differs from the curator’s pin or an earlier install.
                  <span className="block text-muted-foreground">
                    This never overrides a published hash that differs, or a framework that is missing.
                  </span>
                </span>
              </label>
            </div>
          )}

          {list.listing.entries.length === 0 ? (
            <p className="text-sm text-muted-foreground" data-testid="catalog-empty">
              There are no curated items for this game yet. They appear here once Agora’s catalog lists one.
            </p>
          ) : (
            <ul className="space-y-3" aria-label="Curated items">
              {list.listing.entries.map((entry) => (
                <CatalogRow
                  key={entry.id}
                  entry={entry}
                  canInstall={instanceId !== ''}
                  instanceId={instanceId}
                  result={results[entry.id]}
                  onPreview={() => act(entry, true)}
                  onInstall={() => act(entry, false)}
                />
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}

interface CatalogRowProps {
  entry: CatalogEntry;
  canInstall: boolean;
  /** The instance the entry installs into; a FOMOD install adds its result to the same one. */
  instanceId: string;
  result: EntryState | undefined;
  onPreview: () => void;
  onInstall: () => void;
}

function CatalogRow({ entry, canInstall, instanceId, result, onPreview, onInstall }: CatalogRowProps) {
  const busy = result?.status === 'busy';
  const previewing = result?.status === 'busy' && result.dryRun;
  const installing = result?.status === 'busy' && !result.dryRun;
  return (
    <li className="space-y-3 rounded-xl border border-border bg-card p-4" data-testid="catalog-entry">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <h3 className="font-semibold">{entry.name}</h3>
          <p className="text-sm text-muted-foreground">
            {entry.author ? `By ${entry.author}. ` : ''}
            {entry.downloadStrategy === 'github_release' ? 'From a GitHub release.' : 'From a direct download.'}
          </p>
          {entry.description && <p className="text-sm">{entry.description}</p>}
          <div className="flex flex-wrap gap-2 pt-1">
            {entry.compatibility.map((compat, index) => (
              <Badge key={index} variant="secondary">
                {compat.stores.join(' / ') || 'any store'} · {compat.gameVersions.join(', ') || 'any version'}
              </Badge>
            ))}
          </div>
          {entry.compatibility.some((compat) => compat.requires.length > 0) && (
            <p className="text-xs text-muted-foreground">
              Needs:{' '}
              {entry.compatibility
                .flatMap((compat) => compat.requires)
                .map((req) => (req.minVersion ? `${req.framework} ${req.minVersion} or newer` : req.framework))
                .join('; ')}
            </p>
          )}
        </div>
        <div className="flex shrink-0 gap-2">
          <Button variant="outline" size="sm" onClick={onPreview} disabled={!canInstall || busy}>
            {previewing ? 'Checking…' : 'Preview'}
          </Button>
          <Button size="sm" onClick={onInstall} disabled={!canInstall || busy}>
            {installing ? 'Installing…' : 'Install'}
          </Button>
        </div>
      </div>

      {result?.status === 'error' && (
        <p role="alert" className="text-sm text-destructive">
          Not installed: {result.message}
        </p>
      )}

      {result?.status === 'done' && <CatalogResult reply={result.reply} instanceId={instanceId} />}
    </li>
  );
}

function CatalogResult({ reply, instanceId }: { reply: CatalogInstallReply; instanceId: string }) {
  // The wizard's state. This component only renders once the install is done, and a new install
  // unmounts it first, so each run starts with the wizard open.
  const [wizard, setWizard] = useState<'open' | 'closed' | 'installed'>('open');
  const size = reply.size !== null ? ` (${formatBytes(reply.size)})` : '';
  const frameworkNotes = reply.frameworks.map((framework) =>
    framework.foundVersion
      ? `${framework.name} ${framework.foundVersion}`
      : `${framework.name} is installed, but its version cannot be read`,
  );
  const warnings = reply.warnings.map((warning) => (
    <li key={warning} className="text-amber-700 dark:text-amber-300">
      {warning}
    </li>
  ));

  switch (reply.state) {
    case 'dry_run':
      return (
        <div className="space-y-1 text-sm" data-testid="catalog-preview">
          <p>
            Would download {reply.fileName}{size} from {reply.source}. Nothing has been downloaded.
          </p>
          {frameworkNotes.length > 0 && <p className="text-muted-foreground">Checked: {frameworkNotes.join('; ')}.</p>}
          {warnings.length > 0 && <ul className="space-y-1">{warnings}</ul>}
        </div>
      );
    case 'already_installed':
      return (
        <p className="text-sm" data-testid="catalog-already">
          {reply.sameRelease
            ? `Already in this instance${reply.installedRelease ? ` (release ${reply.installedRelease})` : ''}. Nothing changed.`
            : `This instance has release ${reply.installedRelease ?? 'an earlier one'}; this entry offers ${reply.release ?? 'another release'}. Nothing changed.`}
        </p>
      );
    case 'placed': {
      const outcome = hashOutcome(reply.hash);
      return (
        <div className="space-y-2 text-sm" data-testid="catalog-placed">
          <p>
            Installed {reply.itemName} into {reply.mountPath}.
          </p>
          <div className="rounded-lg border border-border p-3" data-testid="catalog-hash">
            <p className={outcome.tone === 'verified' ? 'font-medium text-emerald-700 dark:text-emerald-300' : 'font-medium'}>
              {outcome.headline}
            </p>
            <p className="text-muted-foreground">{outcome.basis}</p>
            {outcome.acceptedPast && <p className="text-amber-700 dark:text-amber-300">{outcome.acceptedPast}</p>}
            <p className="font-mono text-xs text-muted-foreground">sha-256 {reply.hash.sha256.slice(0, 16)}…</p>
          </div>
          {warnings.length > 0 && <ul className="space-y-1">{warnings}</ul>}
        </div>
      );
    }
    case 'needs_installer':
      return (
        <div className="space-y-3 text-sm" data-testid="catalog-needs-installer">
          <p>
            {reply.itemName} has an installer with questions. Answer them to finish the install. {reply.reason}
          </p>
          {wizard === 'open' && (
            <FomodWizard
              contentId={reply.contentItemId}
              instanceId={instanceId}
              onClose={() => setWizard('closed')}
              onInstalled={() => setWizard('installed')}
            />
          )}
          {wizard === 'closed' && (
            <p data-testid="catalog-installer-closed">
              The installer was closed. The archive stays in your content library, and nothing has been added to the
              instance yet.
            </p>
          )}
          {wizard === 'installed' && (
            <p data-testid="catalog-installer-done">The installer finished. The wizard reports what was added.</p>
          )}
        </div>
      );
  }
}
