import { useEffect, useState } from 'react';
import { msaOpenVerificationUrl } from '../lib/tauri';
import type { MsaLoginPrompt } from '../lib/tauri';

interface MsaDeviceFlowPanelProps {
  prompt: MsaLoginPrompt;
  polling: boolean;
  onCancel: () => void;
  className?: string;
}

function remainingLabel(expiresAt: string, now: number): string | null {
  const expiry = Date.parse(expiresAt);
  if (Number.isNaN(expiry)) return null;
  const seconds = Math.floor((expiry - now) / 1000);
  if (seconds <= 0) return null;
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}

/**
 * Microsoft device-code sign-in instructions.
 *
 * The browser is deliberately NOT opened when the flow starts: Microsoft asks
 * the user to type a code that only exists in this window, so stealing focus
 * before they have it in hand means alt-tabbing back to read it. Opening the
 * page is a backend command rather than a frontend URL handoff, so the
 * verification URL the shell receives is the one Microsoft returned and not a
 * string this component could be tricked into supplying.
 */
export function MsaDeviceFlowPanel({
  prompt,
  polling,
  onCancel,
  className = '',
}: MsaDeviceFlowPanelProps) {
  const [handedOff, setHandedOff] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  const copy = (value: string) => navigator.clipboard.writeText(value);

  const openBrowser = () => {
    setHandedOff(true);
    msaOpenVerificationUrl().catch(() => {
      // Best-effort: the URL stays on screen for manual entry.
    });
  };

  // Copy first, hand off second. A clipboard failure still opens the browser —
  // the code is visible above and can be typed in.
  const copyCodeAndContinue = () => {
    copy(prompt.user_code).catch(() => {}).finally(openBrowser);
  };

  const remaining = remainingLabel(prompt.expires_at, now);

  return (
    <div className={`rounded-lg border border-border bg-muted p-3 space-y-2 ${className}`}>
      <p className="text-xs">
        {handedOff
          ? 'Code copied. Enter it on the Microsoft page in your browser — we brought it to the front for you.'
          : 'Copy the code below, then we will open the Microsoft sign-in page in your browser.'}
      </p>
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span>Code:</span>
        <span className="font-mono font-bold tracking-widest">{prompt.user_code}</span>
      </div>
      <p className="text-sm font-semibold text-primary break-all">{prompt.verification_uri}</p>
      {remaining ? (
        <p className="text-xs text-muted-foreground">Code is valid for {remaining}.</p>
      ) : (
        <p className="text-xs text-muted-foreground">
          This code has expired. Start the sign-in again to get a new one.
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={copyCodeAndContinue}
          className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90"
        >
          {handedOff ? 'Copy code & reopen browser' : 'Copy code & open browser'}
        </button>
        <button
          type="button"
          onClick={openBrowser}
          className="rounded-lg border border-border px-3 py-1.5 text-xs font-medium hover:bg-accent"
        >
          Open in browser
        </button>
        <button
          type="button"
          onClick={() => copy(prompt.verification_uri).catch(() => {})}
          className="rounded-lg border border-border px-3 py-1.5 text-xs font-medium hover:bg-accent"
        >
          Copy URL
        </button>
        {polling && (
          <button
            type="button"
            onClick={onCancel}
            className="rounded-lg border border-border px-3 py-1.5 text-xs font-medium hover:bg-accent"
          >
            Cancel
          </button>
        )}
      </div>
      {polling && <p className="text-xs text-muted-foreground">Waiting for you to sign in…</p>}
    </div>
  );
}
