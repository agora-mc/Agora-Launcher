import { useEffect, useState } from 'react';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import { explainModPresence, formatError, type PresenceExplanation } from '@/lib/tauri';

/**
 * "Why is this mod here?" — traces an unfamiliar jar back to the mod the user
 * actually asked for.
 *
 * Launchers show what a mod depends on; almost none show the inverse, which is
 * the direction someone is looking when they find a jar they don't recognize.
 */
export function WhyInstalledDialog({
  instanceId,
  filename,
  displayName,
  onClose,
}: {
  instanceId: string;
  filename: string;
  displayName: string;
  onClose: () => void;
}) {
  const [explanation, setExplanation] = useState<PresenceExplanation | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    explainModPresence(instanceId, filename)
      .then((result) => {
        if (cancelled) return;
        setExplanation(result);
        setError(null);
      })
      .catch((e) => { if (!cancelled) setError(formatError(e)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [instanceId, filename]);

  const summary = (() => {
    if (!explanation) return null;
    if (explanation.pack_managed) return 'This came from the instance’s modpack.';
    if (!explanation.installed_as_dependency) return 'You installed this directly.';
    if (explanation.orphaned) {
      return 'Agora installed this as a dependency, and nothing needs it any more.';
    }
    return 'Agora installed this to satisfy another mod’s dependency.';
  })();

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="max-w-lg">
        <DialogTitle>Why is {displayName} here?</DialogTitle>
        <DialogDescription>{filename}</DialogDescription>

        {loading && <p className="mt-4 text-sm text-muted-foreground">Tracing dependencies…</p>}
        {error && <p className="mt-4 text-sm text-destructive">{error}</p>}

        {explanation && (
          <div className="mt-4 space-y-4">
            <p className="text-sm">{summary}</p>

            {explanation.root_paths.length > 0 && (
              <div>
                <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                  Required by
                </h4>
                <ul className="mt-2 space-y-1">
                  {explanation.root_paths.map((path) => (
                    <li key={path.join('>')} className="text-sm">
                      {path.map((step, index) => (
                        <span key={step}>
                          {index > 0 && <span className="text-muted-foreground"> → </span>}
                          <span className={index === path.length - 1 ? 'font-medium' : undefined}>
                            {step}
                          </span>
                        </span>
                      ))}
                    </li>
                  ))}
                </ul>
              </div>
            )}

            {explanation.dependents.length > 0 && (
              <div>
                <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                  Directly depended on by
                </h4>
                <ul className="mt-2 space-y-1">
                  {explanation.dependents.map((dependent) => (
                    <li key={dependent.filename} className="text-sm">
                      {dependent.filename}
                      <span className="ml-2 text-xs text-muted-foreground">
                        {dependent.requirement === 'required' ? 'required' : 'optional'}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            )}

            {explanation.dependents.length === 0 && explanation.root_paths.length === 0 && (
              <p className="text-sm text-muted-foreground">
                Nothing installed declares a dependency on it.
              </p>
            )}
          </div>
        )}

        <div className="mt-6 flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg border border-input bg-background px-4 py-2 text-sm font-medium hover:bg-accent"
          >
            Close
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
