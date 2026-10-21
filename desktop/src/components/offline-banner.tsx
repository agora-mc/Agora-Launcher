import { useCallback, useEffect, useState } from 'react';
import { WifiOff } from 'lucide-react';
import { useOfflineStatus } from '@/hooks/use-offline-status';

const DISMISSED_KEY = 'offline_banner_dismissed';

/**
 * Offline Mode banner — renders a slim amber top-of-viewport bar
 * only when the browser reports offline.  Dismissible for the
 * session (persisted in sessionStorage).
 */
export function OfflineBanner() {
  const [dismissed, setDismissed] = useState(false);

  const { isOnline } = useOfflineStatus();

  // When connectivity returns, clear the dismissed flag so the banner
  // can reappear next time the user goes offline.
  useEffect(() => {
    if (isOnline) {
      sessionStorage.removeItem(DISMISSED_KEY);
      setDismissed(false);
    }
  }, [isOnline]);

  const handleDismiss = useCallback(() => {
    setDismissed(true);
    sessionStorage.setItem(DISMISSED_KEY, '1');
  }, []);

  if (isOnline || dismissed) {
    return null;
  }

  return (
    <div
      className="fixed left-0 right-0 top-0 z-30 flex items-center justify-between gap-3 border-b px-4 py-2 text-sm shadow-lg"
      style={{
        backgroundColor: 'rgb(var(--amber-900))',
        borderColor: 'rgb(var(--amber-700))',
        color: 'rgb(var(--amber-100))',
      }}
      role="status"
      aria-live="polite"
    >
      <div className="flex items-center gap-2">
        <WifiOff className="h-4 w-4 shrink-0" />
        <span className="text-xs leading-snug">
          You&apos;re offline — Agora is running in Offline Mode. Cached catalog
          and local instances remain available.
        </span>
      </div>

      <div className="flex items-center gap-2">
        <button
          onClick={() => {
            window.dispatchEvent(new CustomEvent('agora-navigate', { detail: 'settings' }));
          }}
          className="rounded-md bg-amber-600 px-2.5 py-1 text-xs font-medium text-white hover:bg-amber-500 transition-colors"
        >
          View Privacy settings
        </button>

        <button
          onClick={handleDismiss}
          className="ml-1 rounded-sm p-0.5 text-amber-300 hover:text-amber-100 hover:bg-amber-800/50 transition-colors"
          aria-label="Dismiss offline banner"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>
    </div>
  );
}
