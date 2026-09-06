import { useMemo, useState } from 'react';
import { HardDrive } from 'lucide-react';
import { SettingsSection } from './SettingsSection';
import {
  formatError,
  runRuntimePrune,
  scanRuntimePrune,
  type PruneCategory,
  type PruneReport,
} from '@/lib/tauri';
import { useConfirm } from '@/components/ui/confirm';

const CATEGORY_LABELS: Record<PruneCategory, string> = {
  libraries: 'Libraries',
  assets: 'Game assets',
  natives: 'Native binaries',
  versions: 'Version files',
  java_runtimes: 'Java runtimes',
  logging: 'Logging configs',
};

function formatBytes(bytes: number): string {
  if (bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** exponent;
  return `${value >= 10 || exponent === 0 ? Math.round(value) : value.toFixed(1)} ${units[exponent]}`;
}

/**
 * Reclaim disk from the shared `minecraft-runtime/` tree.
 *
 * Instance cleanup only ever frees mods; the multi-gigabyte reclaim lives in
 * the shared libraries, assets and Java runtimes that no instance uses any
 * more. Scanning is always a dry run, and the scan fails closed — when it
 * cannot confidently prove something is unused it reports nothing and says why,
 * which is why a zero result with warnings is a normal outcome rather than a
 * bug.
 */
export function RuntimeReclaim() {
  const { confirm } = useConfirm();
  const [report, setReport] = useState<PruneReport | null>(null);
  const [selected, setSelected] = useState<Set<PruneCategory>>(new Set());
  const [busy, setBusy] = useState<'scan' | 'prune' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [freed, setFreed] = useState<string | null>(null);

  const reclaimable = useMemo(
    () => (report?.categories ?? []).filter((entry) => entry.file_count > 0),
    [report],
  );
  const selectedBytes = reclaimable
    .filter((entry) => selected.has(entry.category))
    .reduce((sum, entry) => sum + entry.total_bytes, 0);

  const scan = async () => {
    setBusy('scan');
    setError(null);
    setFreed(null);
    try {
      const result = await scanRuntimePrune();
      setReport(result);
      // Preselect everything reclaimable: the user still has to confirm, and
      // unchecking is less work than checking six boxes.
      setSelected(new Set(result.categories.filter((c) => c.file_count > 0).map((c) => c.category)));
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(null);
    }
  };

  const reclaim = async () => {
    const categories = [...selected];
    if (categories.length === 0) return;
    if (!await confirm({
      title: `Permanently delete ${formatBytes(selectedBytes)} of unused runtime files?`,
      confirmLabel: 'Delete files',
      tone: 'danger',
    })) return;
    setBusy('prune');
    setError(null);
    try {
      const result = await runRuntimePrune(categories);
      setFreed(`Freed ${formatBytes(result.total_freed_bytes)} across ${result.total_freed_files} files.`);
      setReport(null);
      setSelected(new Set());
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <SettingsSection
      id="settings-storage"
      icon={HardDrive}
      title="Reclaim disk space"
      description="Remove shared Minecraft files that no instance uses any more."
    >
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={() => void scan()}
          disabled={busy !== null}
          className="rounded-lg border border-input bg-background px-4 py-2 text-sm font-medium hover:bg-accent disabled:opacity-50"
        >
          {busy === 'scan' ? 'Scanning…' : 'Scan for unused files'}
        </button>
        {report && reclaimable.length > 0 && (
          <button
            type="button"
            onClick={() => void reclaim()}
            disabled={busy !== null || selected.size === 0}
            className="rounded-lg bg-destructive px-4 py-2 text-sm font-medium text-destructive-foreground hover:bg-destructive/90 disabled:opacity-50"
          >
            {busy === 'prune' ? 'Reclaiming…' : `Reclaim ${formatBytes(selectedBytes)}`}
          </button>
        )}
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}
      {freed && <p className="text-sm text-primary">{freed}</p>}

      {report && reclaimable.length === 0 && (
        <p className="text-sm text-muted-foreground">Nothing to reclaim — everything on disk is in use.</p>
      )}

      {reclaimable.length > 0 && (
        <ul className="space-y-1">
          {reclaimable.map((entry) => (
            <li key={entry.category}>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={selected.has(entry.category)}
                  disabled={busy !== null}
                  onChange={(e) => setSelected((prev) => {
                    const next = new Set(prev);
                    if (e.target.checked) next.add(entry.category);
                    else next.delete(entry.category);
                    return next;
                  })}
                  className="h-4 w-4 accent-primary"
                />
                <span>{CATEGORY_LABELS[entry.category] ?? entry.category}</span>
                <span className="ml-auto text-xs text-muted-foreground">
                  {entry.file_count} file{entry.file_count === 1 ? '' : 's'} · {formatBytes(entry.total_bytes)}
                </span>
              </label>
            </li>
          ))}
        </ul>
      )}

      {report && report.warnings.length > 0 && (
        <details className="text-xs text-muted-foreground">
          <summary className="cursor-pointer">
            {report.warnings.length} item{report.warnings.length === 1 ? '' : 's'} left untouched to be safe
          </summary>
          <ul className="mt-2 space-y-1">
            {report.warnings.map((warning) => (
              <li key={warning} className="break-words">{warning}</li>
            ))}
          </ul>
        </details>
      )}
    </SettingsSection>
  );
}
