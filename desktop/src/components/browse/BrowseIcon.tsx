import { useState } from 'react';

export function BrowseIcon({ iconUrl, name, className = '' }: { iconUrl: string | null; name: string; className?: string }) {
  const [failed, setFailed] = useState(false);

  if (!iconUrl || failed) {
    return (
      <div className={`browse-icon browse-icon--placeholder ${className}`} aria-hidden>
        {name.trim().charAt(0).toLocaleUpperCase() || '?'}
      </div>
    );
  }

  return (
    <img
      src={iconUrl}
      alt=""
      className={`browse-icon ${className}`}
      loading="lazy"
      decoding="async"
      onError={() => setFailed(true)}
    />
  );
}
