import { useState } from 'react';
import { open as openUrl } from '@tauri-apps/plugin-shell';
import type { DeviceFlowResponse } from '../lib/tauri';

interface DeviceFlowPanelProps {
  device: DeviceFlowResponse;
  polling: boolean;
  onCancel: () => void;
  className?: string;
}

/**
 * GitHub device-flow instructions.
 *
 * The browser is deliberately NOT opened when the flow starts. GitHub asks the
 * user to type a code that only exists in this window, so stealing focus before
 * they have it in hand means alt-tabbing back to read it. The panel keeps focus
 * here until the code is on the clipboard, then hands off to the browser. The
 * "Open in browser" button stays available for anyone who copies by hand or
 * whose clipboard is unavailable.
 */
export function DeviceFlowPanel({ device, polling, onCancel, className = '' }: DeviceFlowPanelProps) {
  const [handedOff, setHandedOff] = useState(false);

  const copy = (value: string) => navigator.clipboard.writeText(value);

  const openBrowser = () => {
    setHandedOff(true);
    try {
      const p = openUrl(device.verification_uri);
      Promise.resolve(p).catch(() => {
        // Best-effort: the URL stays on screen for manual entry.
      });
    } catch {
      // Best-effort: the URL stays on screen for manual entry.
    }
  };

  // Copy first, hand off second. A clipboard failure still opens the browser —
  // the code is visible above and can be typed in.
  const copyCodeAndContinue = () => {
    copy(device.user_code).catch(() => {}).finally(openBrowser);
  };

  return (
    <div className={`rounded-lg border border-border bg-muted p-3 space-y-2 ${className}`}>
      <p className="text-xs">
        {handedOff
          ? 'Code copied. Paste it into GitHub in your browser — we brought it to the front for you.'
          : 'Copy the code below, then we will open GitHub in your browser.'}
      </p>
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span>Code:</span>
        <span className="font-mono font-bold tracking-widest">{device.user_code}</span>
      </div>
      <p className="text-sm font-semibold text-primary break-all">{device.verification_uri}</p>
      {device.expires_in > 0 && (
        <p className="text-xs text-muted-foreground">
          Code is valid for {Math.floor(device.expires_in / 60)}:{String(device.expires_in % 60).padStart(2, '0')}.
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
          onClick={() => copy(device.verification_uri).catch(() => {})}
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
      {polling && <p className="text-xs text-muted-foreground">Waiting for authorization…</p>}
    </div>
  );
}
