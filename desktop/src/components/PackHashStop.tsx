import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog';

/**
 * A .mrpack import stopped because files do not match the hashes the pack lists
 * (or it lists none). Nothing was imported. Every file is named, and Install
 * anyway imports exactly those as they are.
 */
export function PackHashStop({
  label,
  message,
  onInstallAnyway,
  onCancel,
}: {
  label: string;
  message: string;
  onInstallAnyway: () => void;
  onCancel: () => void;
}) {
  return (
    <Dialog open onOpenChange={(open) => { if (!open) onCancel(); }}>
      <DialogContent className="max-w-lg" data-testid="pack-hash-stop">
        <DialogTitle>{label} did not pass its hash check</DialogTitle>
        <DialogDescription>
          Nothing was imported. Install anyway only if you trust this pack.
        </DialogDescription>
        <p role="alert" className="max-h-64 overflow-y-auto whitespace-pre-wrap break-words rounded-lg border border-amber-500/50 bg-amber-500/10 p-3 text-xs">
          {message}
        </p>
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-lg border border-input px-3 py-1.5 text-xs font-medium hover:bg-accent"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onInstallAnyway}
            className="rounded-lg border border-destructive/40 px-3 py-1.5 text-xs font-medium text-destructive hover:bg-destructive/10"
          >
            Install anyway
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
