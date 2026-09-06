import * as React from 'react';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';

/**
 * In-app replacements for `window.confirm` and `window.prompt`.
 *
 * The native ones are operating-system modals. A controller cannot touch them
 * at all — they are not in the page, so there is nothing to focus and nothing
 * the Gamepad API can see — and they block the JavaScript thread while they are
 * up. Twenty of them stood between a controller user and ordinary actions like
 * deleting an instance or renaming one.
 *
 * These render as ordinary dialogs, which means they inherit controller support
 * from `DialogContent` for free, look like the rest of Agora, and can carry
 * more than a single line of plain text.
 *
 * The API is promise-based so call sites read almost identically to the ones
 * they replace:
 *
 *     if (!await confirm({ title: 'Delete this instance?' })) return;
 */

export interface ConfirmOptions {
  title: string;
  /** Optional detail shown under the title. */
  body?: React.ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  /** `danger` styles the confirm action as destructive. */
  tone?: 'default' | 'danger';
}

export interface PromptOptions {
  title: string;
  body?: React.ReactNode;
  initialValue?: string;
  placeholder?: string;
  confirmLabel?: string;
  cancelLabel?: string;
}

interface ConfirmRequest {
  kind: 'confirm';
  options: ConfirmOptions;
  resolve: (value: boolean) => void;
}

interface PromptRequest {
  kind: 'prompt';
  options: PromptOptions;
  resolve: (value: string | null) => void;
}

type Request = ConfirmRequest | PromptRequest;

interface ConfirmApi {
  confirm: (options: ConfirmOptions) => Promise<boolean>;
  prompt: (options: PromptOptions) => Promise<string | null>;
}

const ConfirmContext = React.createContext<ConfirmApi | null>(null);

/**
 * Falls back to resolving negatively rather than throwing when no provider is
 * mounted. A missing provider must not turn "are you sure?" into a crash in the
 * middle of someone's action — and it must never mean "yes".
 */
const FALLBACK: ConfirmApi = {
  confirm: async () => false,
  prompt: async () => null,
};

export function useConfirm(): ConfirmApi {
  return React.useContext(ConfirmContext) ?? FALLBACK;
}

export function ConfirmProvider({ children }: { children: React.ReactNode }) {
  const [request, setRequest] = React.useState<Request | null>(null);
  const [value, setValue] = React.useState('');

  const api = React.useMemo<ConfirmApi>(() => ({
    confirm: (options) => new Promise<boolean>((resolve) => {
      setRequest({ kind: 'confirm', options, resolve });
    }),
    prompt: (options) => new Promise<string | null>((resolve) => {
      setValue(options.initialValue ?? '');
      setRequest({ kind: 'prompt', options, resolve });
    }),
  }), []);

  // Settle on close however it happened — Escape, the backdrop, the close
  // button, or a controller's Cancel. An unsettled promise here would leave the
  // caller waiting forever mid-action.
  const settle = React.useCallback((confirmed: boolean) => {
    setRequest((current) => {
      if (!current) return null;
      if (current.kind === 'confirm') current.resolve(confirmed);
      else current.resolve(confirmed ? value : null);
      return null;
    });
  }, [value]);

  const options = request?.options;
  const danger = request?.kind === 'confirm' && request.options.tone === 'danger';

  return (
    <ConfirmContext.Provider value={api}>
      {children}
      <Dialog open={request !== null} onOpenChange={(open) => { if (!open) settle(false); }}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{options?.title ?? ''}</DialogTitle>
            {options?.body ? (
              <DialogDescription asChild>
                <div className="whitespace-pre-line text-sm text-muted-foreground">{options.body}</div>
              </DialogDescription>
            ) : (
              <DialogDescription className="sr-only">Confirm this action.</DialogDescription>
            )}
          </DialogHeader>

          {request?.kind === 'prompt' && (
            <Input
              autoFocus
              value={value}
              placeholder={request.options.placeholder}
              onChange={(event) => setValue(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  settle(true);
                }
              }}
            />
          )}

          <DialogFooter>
            <button
              type="button"
              onClick={() => settle(false)}
              className="rounded-lg border border-input px-4 py-2 text-sm font-medium hover:bg-accent focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
            >
              {options?.cancelLabel ?? 'Cancel'}
            </button>
            <button
              type="button"
              onClick={() => settle(true)}
              className={cn(
                'rounded-lg px-4 py-2 text-sm font-semibold focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring',
                danger
                  ? 'bg-destructive text-destructive-foreground hover:bg-destructive/90'
                  : 'bg-primary text-primary-foreground hover:bg-primary/90',
              )}
            >
              {options?.confirmLabel ?? 'Confirm'}
            </button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </ConfirmContext.Provider>
  );
}
