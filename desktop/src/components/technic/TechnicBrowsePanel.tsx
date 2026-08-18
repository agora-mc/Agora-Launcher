import { useEffect, useState, type FormEvent } from 'react';
import { ChevronDown, ChevronUp, Download, Search, ShieldCheck, ShieldOff } from 'lucide-react';
import {
  formatError,
  installTechnicSolderPack,
  installTechnicZipPack,
  technicPackDetail,
  technicSearch,
  type ImportResult,
  type TechnicSearchResult,
} from '../../lib/tauri';
import { showToast } from '../Toast';

interface TechnicBrowsePanelProps {
  enabled: boolean;
  allowUnverifiedPacks: boolean;
}

const WEBSITE = 'https://www.technicpack.net';

/**
 * Self-contained Technic browse + install panel.
 *
 * Technic is kept visually separate from the curated/Modrinth catalog on
 * purpose: it is a consent-based source. Tier Solder (solder) entries are
 * MD5-checked when the API reports an MD5; Tier Zip (zip) entries have no
 * integrity at all and remain non-installable until `allow_unverified_packs`.
 */
export function TechnicBrowsePanel({ enabled, allowUnverifiedPacks }: TechnicBrowsePanelProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<TechnicSearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState<string | null>(null);

  const runSearch = async () => {
    setLoading(true);
    setError(null);
    try {
      const items = await technicSearch(query.trim() || 'popular', 12);
      setResults(items);
    } catch (e) {
      setError(formatError(e));
      setResults([]);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (open) {
      void runSearch();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    void runSearch();
  };

  const install = async (result: TechnicSearchResult) => {
    setInstalling(result.slug);
    setError(null);
    try {
      const detail = await technicPackDetail(result.slug);
      let outcome: ImportResult;
      if (detail.tier === 'solder') {
        const solder = detail.solder;
        const build = detail.recommended_build;
        if (!solder || !build) {
          throw new Error('This Solder pack does not report a usable build to install.');
        }
        outcome = await installTechnicSolderPack(detail.slug, solder, build);
      } else {
        if (!allowUnverifiedPacks) {
          throw new Error('Enable "Allow unverified zip packs" in Settings to install zip packs.');
        }
        const downloadUrl = detail.download_url;
        if (!downloadUrl) {
          throw new Error('This zip pack does not report a download URL.');
        }
        outcome = await installTechnicZipPack(
          detail.title,
          downloadUrl,
          null,
          detail.minecraft ?? '',
          '',
          '',
        );
      }
      showToast(
        `Imported "${outcome.name}"${outcome.minecraft_version ? ` (MC ${outcome.minecraft_version})` : ''} — ${outcome.imported_mods} mods; review before launch.`,
        'success',
      );
    } catch (e) {
      setError(formatError(e));
    } finally {
      setInstalling(null);
    }
  };

  if (!enabled) {
    return null;
  }

  const zipDisabled = !allowUnverifiedPacks;

  return (
    <section className="rounded-xl border border-border bg-card">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-3 p-4 text-left"
        aria-expanded={open}
      >
        {open ? <ChevronUp className="h-5 w-5 text-muted-foreground" /> : <ChevronDown className="h-5 w-5 text-muted-foreground" />}
        <div className="flex-1">
          <p className="text-sm font-semibold">Technic modpacks</p>
          <p className="text-xs text-muted-foreground mt-0.5">
            Third-party packs downloaded from Technic. Solder packs are MD5-checked when reported; zip packs have no
            integrity check{zipDisabled && ' and need "Allow unverified zip packs" to install'}.
          </p>
        </div>
        <a
          href={WEBSITE}
          target="_blank"
          rel="noopener noreferrer"
          className="text-xs font-medium text-primary hover:underline"
          onClick={(event) => event.stopPropagation()}
        >
          Technic ↗
        </a>
      </button>

      {open && (
        <div className="border-t border-border p-4 space-y-4">
          <form onSubmit={handleSubmit} className="flex gap-2">
            <input
              type="text"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search Technic packs…"
              className="flex-1 rounded-lg border border-input bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
            />
            <button
              type="submit"
              className="flex items-center gap-1 rounded-lg bg-primary px-3 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              disabled={loading}
            >
              <Search size={15} /> Search
            </button>
          </form>

          {error && (
            <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">
              {error}
            </div>
          )}

          {loading ? (
            <p className="text-center text-sm text-muted-foreground py-4">Searching Technic…</p>
          ) : results.length === 0 ? (
            <p className="text-center text-sm text-muted-foreground py-4">No packs found.</p>
          ) : (
            <ul className="space-y-2">
              {results.map((result) => {
                const zip = result.tier === 'zip';
                return (
                  <li
                    key={result.slug}
                    className="flex items-start gap-3 rounded-lg border border-border bg-background p-3"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <p className="truncate text-sm font-semibold">{result.title}</p>
                        <span
                          className={[
                            'inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium',
                            zip
                              ? 'bg-amber-500/15 text-amber-600 dark:text-amber-400'
                              : 'bg-emerald-500/15 text-emerald-600 dark:text-emerald-400',
                          ].join(' ')}
                          title={zip
                            ? 'Bare zip — no integrity information'
                            : 'Solder — mods MD5-checked when the API reports an MD5'}
                        >
                          {zip ? <ShieldOff size={11} /> : <ShieldCheck size={11} />}
                          {zip ? 'Zip · unverified' : 'Solder · MD5'}
                        </span>
                      </div>
                      {result.description && (
                        <p className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">{result.description}</p>
                      )}
                      <p className="mt-1 text-[11px] text-muted-foreground">
                        {result.installs.toLocaleString()} installs
                        {result.author ? ` · ${result.author}` : ''}
                      </p>
                    </div>
                    <button
                      type="button"
                      onClick={() => void install(result)}
                      disabled={installing === result.slug || (zip && zipDisabled)}
                      className={[
                        'flex items-center gap-1 rounded-lg px-3 py-1.5 text-xs font-medium',
                        zip && zipDisabled
                          ? 'cursor-not-allowed border border-border text-muted-foreground'
                          : 'bg-primary text-primary-foreground hover:bg-primary/90',
                        installing === result.slug ? 'opacity-50' : '',
                      ].join(' ')}
                      title={
                        zip && zipDisabled
                          ? 'Enable "Allow unverified zip packs" in Settings to install this pack.'
                          : 'Install as a new instance'
                      }
                    >
                      {installing === result.slug ? (
                        'Installing…'
                      ) : (
                        <>
                          <Download size={14} /> Install
                        </>
                      )}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
