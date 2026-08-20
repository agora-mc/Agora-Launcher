import type { CSSProperties, ReactNode } from 'react';
import { instanceInitials, instanceTint, loaderHue, normalizeLoader } from '../lib/instanceIdentity';

/**
 * The tile that stands for an instance: its custom image when it has one, a
 * generated placeholder when it does not. See `lib/instanceIdentity.ts` for
 * why the placeholder looks the way it does.
 *
 * Always decorative — every caller sits next to the instance name already, so
 * announcing the tile too would just repeat it.
 */
export function InstanceIcon({
  name,
  seed,
  loader,
  iconSrc,
  size = 56,
  className = '',
}: {
  name: string;
  /** Stable per-instance value (the instance id). Falls back to the name. */
  seed?: string | null;
  loader?: string | null;
  iconSrc?: string | null;
  /** Edge length in px. Drives the initials' size too. */
  size?: number;
  className?: string;
}) {
  const box: CSSProperties = { width: size, height: size };

  if (iconSrc) {
    return (
      <img
        src={iconSrc}
        alt=""
        aria-hidden="true"
        style={box}
        className={`instance-icon instance-icon--image ${className}`.trim()}
      />
    );
  }

  const tint = instanceTint(seed || name, loader);
  const style = {
    ...box,
    fontSize: Math.round(size * 0.36),
    '--instance-icon-hue-a': String(tint.hueA),
    '--instance-icon-hue-b': String(tint.hueB),
    '--instance-icon-angle': `${tint.angle}deg`,
  } as CSSProperties;

  return (
    <span
      aria-hidden="true"
      data-loader={normalizeLoader(loader) || 'unknown'}
      style={style}
      className={`instance-icon instance-icon--generated ${className}`.trim()}
    >
      <span className="instance-icon__initials">{instanceInitials(name)}</span>
    </span>
  );
}

/**
 * The loader/version pill that sits under an instance name. Carries the same
 * loader hue as the tile, so the dot and the tile agree at a glance.
 */
export function LoaderChip({ loader, loaderVersion }: { loader: string; loaderVersion?: string | null }) {
  const key = normalizeLoader(loader);
  const label = key === 'vanilla' || !loaderVersion ? loader : `${loader} ${loaderVersion}`;
  return (
    <span className="instance-chip" style={{ '--instance-chip-hue': String(loaderHue(loader)) } as CSSProperties}>
      <span className="instance-chip__dot" aria-hidden="true" />
      {label}
    </span>
  );
}

/** A neutral sibling of `LoaderChip` for facts with no loader identity. */
export function MetaChip({ children }: { children: ReactNode }) {
  return <span className="instance-chip instance-chip--muted">{children}</span>;
}
