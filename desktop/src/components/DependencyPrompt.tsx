import { useState } from 'react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog';

export interface DependencyPromptCandidate {
  key: string;
  label: string;
  // Lowercase because that is what the backend sends: core derives these enums
  // with #[serde(rename_all = "kebab-case")].
  requirement: 'required' | 'optional';
  source: 'jar' | 'manifest';
  isConflict?: boolean;
}

export interface DependencyPromptProps {
  title: string;
  description?: string;
  actionLabel: string;
  candidates: DependencyPromptCandidate[];
  onConfirm: (selectedKeys: string[]) => void;
  onCancel: () => void;
}

/**
 * DependencyPrompt — modal for reviewing dependent mods before
 * install / remove / disable operations.
 *
 * Required rows are pre-checked; optional rows start unchecked.
 * Conflict rows get a subtle warning border + classification caption.
 */
export function DependencyPrompt({
  title,
  description,
  actionLabel,
  candidates,
  onConfirm,
  onCancel,
}: DependencyPromptProps) {
  const [selected, setSelected] = useState<Set<string>>(
    new Set(candidates.filter((c) => c.requirement === 'required').map((c) => c.key)),
  );

  const toggle = (key: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const hasAny = selected.size > 0;

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onCancel(); }}>
      <DialogContent className="max-w-lg">
        <DialogTitle>{title}</DialogTitle>
        <DialogDescription>
          {description ?? 'Review the dependency effects before continuing.'}
        </DialogDescription>

        <div className="max-h-72 overflow-y-auto space-y-2 mb-4">
          {candidates.map((cand) => {
            const isChecked = selected.has(cand.key);
            return (
              <div
                key={cand.key}
                className={[
                  'rounded-lg border px-3 py-2 text-sm transition-colors',
                  cand.isConflict
                    ? 'border-amber-300 dark:border-amber-700 bg-amber-50/50 dark:bg-amber-900/10'
                    : 'border-border',
                ].join(' ')}
              >
                <div className="flex items-start gap-2">
                  <input
                    type="checkbox"
                    checked={isChecked}
                    onChange={() => toggle(cand.key)}
                    className="mt-1 h-4 w-4 rounded border-input text-primary focus:ring-primary"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="font-medium truncate">{cand.label}</span>
                      <span
                        className={[
                          'rounded-full px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide',
                          cand.requirement === 'required'
                            ? 'bg-destructive/10 text-destructive dark:bg-destructive/20 dark:text-destructive'
                            : 'bg-muted text-muted-foreground',
                        ].join(' ')}
                      >
                        {cand.requirement === 'required' ? 'Required' : 'Optional'}
                      </span>
                      <span
                        className={[
                          'rounded-full px-1.5 py-0.5 text-[10px] font-medium',
                          cand.source === 'jar'
                            ? 'bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-300'
                            : 'bg-muted text-muted-foreground',
                        ].join(' ')}
                      >
                        {cand.source === 'jar' ? 'from jar' : 'from manifest'}
                      </span>
                      {cand.source === 'jar' && (
                        <span className="rounded-full bg-primary px-1.5 py-0.5 text-[10px] font-medium text-primary-foreground">
                          Recommended
                        </span>
                      )}
                    </div>
                    {cand.isConflict && (
                      <p className="text-[10px] text-amber-700 dark:text-amber-300 mt-1">
                        Conflict: {cand.source === 'jar' ? 'Jar' : 'Manifest'} classified as {cand.requirement === 'required' ? 'Required' : 'Optional'}, but the opposite source has a different classification.
                      </p>
                    )}
                    {!isChecked && cand.requirement === 'required' && (
                      <p className="text-[10px] text-amber-600 dark:text-amber-400 mt-1">
                        Warning: Disabling may cause crashes.
                      </p>
                    )}
                  </div>
                </div>
              </div>
            );
          })}
        </div>

        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent"
          >
            Cancel
          </button>
          <button
            onClick={() => onConfirm(Array.from(selected))}
            disabled={!hasAny}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {actionLabel}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
