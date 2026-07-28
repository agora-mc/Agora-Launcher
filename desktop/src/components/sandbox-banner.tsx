import { useEffect, useState } from 'react';
import { getGovernanceConfig, type GovernanceConfig } from '../lib/tauri';

export function SandboxBanner() {
  const [config, setConfig] = useState<GovernanceConfig | null>(null);

  useEffect(() => {
    let cancelled = false;
    getGovernanceConfig()
      .then((c) => {
        if (!cancelled) setConfig(c);
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, []);

  if (!config) return null;

  const isSandbox = config.environment === 'sandbox';
  const isDevReg = config.development_registry;

  if (!isSandbox && !isDevReg) return null;

  let bannerText = '';
  if (isSandbox) {
    bannerText = 'Sandbox governance active - ' + config.repository;
  }
  if (isDevReg) {
    if (isSandbox) {
      bannerText += ' | Development registry';
    } else {
      bannerText = 'Development registry - ' + config.repository;
    }
  }

  // Offset below OfflineBanner when user is offline
  const isOffline = typeof navigator !== 'undefined' && !navigator.onLine;

  return (
    <div
      className="fixed left-0 right-0 z-30 flex items-center border-b px-4 py-2 text-sm shadow-lg"
      style={{
        top: isOffline ? '38px' : '0px',
        backgroundColor: 'rgb(var(--amber-900))',
        borderColor: 'rgb(var(--amber-700))',
        color: 'rgb(var(--amber-100))',
      }}
      role="status"
      aria-live="polite"
    >
      <span className="text-xs leading-snug truncate">{bannerText}</span>
    </div>
  );
}
