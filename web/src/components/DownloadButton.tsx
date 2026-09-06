'use client';

import { useState, useEffect } from 'react';
import { GITHUB_API_RELEASES_URL, GITHUB_RELEASES_URL } from '@/lib/site';
import {
  pickAsset,
  selectLatestDesktopRelease,
  toDesktopRelease,
  type DesktopRelease,
  type DetectedOS,
  type ReleaseAsset,
} from '@/lib/releases';

function detectOS(): DetectedOS {
  if (typeof navigator === 'undefined') return 'unknown';
  const ua = navigator.userAgent;
  if (ua.includes('Win')) return 'windows';
  if (ua.includes('Mac')) return 'macos';
  if (ua.includes('Linux')) return 'linux';
  return 'unknown';
}

const OS_INFO: Record<DetectedOS, { label: string; icon: string }> = {
  windows: { label: 'Windows', icon: '🪟' },
  macos: { label: 'macOS', icon: '🍎' },
  linux: { label: 'Linux', icon: '🐧' },
  unknown: { label: '', icon: '⬇' },
};

interface DownloadButtonProps {
  /**
   * The newest desktop release as of `next build`. The browser re-checks on
   * mount so a release cut after the last site build still wins, but this is
   * what the button offers when that check cannot happen — a rate-limited or
   * blocked GitHub API used to degrade to a bare "all releases" link.
   */
  initialRelease?: DesktopRelease | null;
}

export function DownloadButton({ initialRelease = null }: DownloadButtonProps) {
  const [os, setOs] = useState<DetectedOS>('unknown');
  const [release, setRelease] = useState<DesktopRelease | null>(initialRelease);
  // Nothing to wait for when the release was resolved at build time.
  const [loading, setLoading] = useState(initialRelease === null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setOs(detectOS());
    fetch(GITHUB_API_RELEASES_URL)
      .then((res) => {
        if (!res.ok) throw new Error(`${res.status}`);
        return res.json();
      })
      .then((data: unknown) => {
        const latest = toDesktopRelease(selectLatestDesktopRelease(data));
        if (latest) setRelease(latest);
        setLoading(false);
      })
      .catch(() => {
        // Keep whatever the build baked in; only report a failure if that
        // left us with nothing to offer.
        setFailed(initialRelease === null);
        setLoading(false);
      });
  }, [initialRelease]);

  const asset: ReleaseAsset | null = pickAsset(os, release?.assets ?? []);
  const osInfo = OS_INFO[os];
  const label = loading
    ? 'Download Agora'
    : osInfo.label
    ? `Download for ${osInfo.label}`
    : 'Download Agora';
  const href = asset?.browser_download_url || GITHUB_RELEASES_URL;

  return (
    <div className="flex flex-col items-center gap-2">
      <a
        href={href}
        className="btn-gold px-5 py-3"
        target={asset ? '_blank' : undefined}
        rel={asset ? 'noopener noreferrer' : undefined}
      >
        {osInfo.icon} {label}
        {asset?.size ? ` · ${(asset.size / 1048576).toFixed(1)} MB` : ''}
      </a>
      <div className="flex flex-col items-center gap-1">
        {release?.tag && (
          <p className="text-xs text-indigo-100/70">
            Latest release <span className="font-semibold text-indigo-100">{release.tag}</span>
          </p>
        )}
        <a
          href={GITHUB_RELEASES_URL}
          className="text-sm text-indigo-100 hover:text-white hover:underline"
          target="_blank"
          rel="noopener noreferrer"
        >
          All platforms →
        </a>
        <p className="text-xs text-indigo-100/60">
          Ignore releases tagged <code className="text-indigo-100/80">registry-*</code> — download the app file for your OS.
        </p>
      </div>
      {failed && (
        <p className="text-center text-xs text-indigo-100/70" role="status">
          We couldn&apos;t check for the latest installer. Browse all desktop downloads instead.
        </p>
      )}
      {!failed && !asset && !loading && (
        <p className="text-center text-xs text-indigo-100/70">
          ⚠️ We couldn&apos;t find a download for your platform. On the releases page, download the file for your platform (<code>.msi</code>, <code>.dmg</code>, or <code>.AppImage</code>). Ignore releases tagged <code>registry-*</code> — those are database updates, not the app itself.
        </p>
      )}
    </div>
  );
}
