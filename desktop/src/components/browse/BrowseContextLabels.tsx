import { ArrowDown, ArrowUp } from 'lucide-react';
import type { BrowseItem, BrowseItemContext } from './types';

export function CuratedBadge() {
  return (
    <span className="browse-curated-badge">
      Curated
    </span>
  );
}

export function BrowseContextLabels({ context }: { context: BrowseItemContext | null }) {
  if (!context || (!context.compatibility && !context.installed && !context.updateAvailable)) return null;
  return (
    <div className="browse-context-labels">
      {context?.compatibility === 'compatible' && (
        <span className="browse-context-label browse-context-label--compatible">
          Compatible with {context.instanceName} · {context.loader} · MC {context.minecraftVersion}
        </span>
      )}
      {context?.compatibility === 'major_match' && (
        <span className="browse-context-label browse-context-label--partial">
          May work with {context.instanceName} · same major Minecraft version
        </span>
      )}
      {context?.installed && (
        <span className="browse-context-label browse-context-label--installed">Installed</span>
      )}
      {context?.updateAvailable && (
        <span className="browse-context-label browse-context-label--partial">Update available</span>
      )}
    </div>
  );
}

export function BrowseStats({ item }: { item: BrowseItem }) {
  if (item.source === 'curated' && item.netScore !== null) {
    return (
      <p className="browse-stats" aria-label={`${item.upvotes ?? 0} upvotes, ${item.downvotes ?? 0} downvotes, score ${item.netScore}`}>
        <span title={`${item.upvotes ?? 0} upvotes`}>
          <ArrowUp aria-hidden size={13} /> {item.upvotes ?? 0}
        </span>
        <span title={`${item.downvotes ?? 0} downvotes`}>
          <ArrowDown aria-hidden size={13} /> {item.downvotes ?? 0}
        </span>
        <span>Score {item.netScore}</span>
      </p>
    );
  }

  const values: string[] = [];
  if (item.downloads !== null) values.push(`${item.downloads.toLocaleString()} downloads`);
  if (item.follows !== null) values.push(`${item.follows.toLocaleString()} followers`);
  return values.length > 0 ? <p className="browse-stats">{values.join(' · ')}</p> : null;
}

export function BrowseVersions({ item }: { item: BrowseItem }) {
  if (item.supportedVersions.length === 0) return null;
  const stableVersions = item.supportedVersions
    .filter((version) => /^\d+\.\d+(?:\.\d+)?$/.test(version))
    .sort((left, right) => left.localeCompare(right, undefined, { numeric: true }));
  let summary: string;
  if (item.supportedVersions.length > 6 && stableVersions.length >= 2) {
    summary = `${stableVersions[0]}–${stableVersions[stableVersions.length - 1]} · ${item.supportedVersions.length} supported versions`;
  } else if (item.supportedVersions.length > 4) {
    summary = `${item.supportedVersions.slice(0, 2).join(', ')} · ${item.supportedVersions.length} supported versions`;
  } else {
    summary = item.supportedVersions.join(', ');
  }
  return (
    <p className="browse-versions" title={item.supportedVersions.join(', ')}>
      MC {summary}
    </p>
  );
}
