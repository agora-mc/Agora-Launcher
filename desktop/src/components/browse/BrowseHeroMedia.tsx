import { useState } from 'react';
import { BrowseIcon } from './BrowseIcon';
import type { BrowseItem } from './types';

export function BrowseHeroMedia({ item }: { item: BrowseItem }) {
  const [heroFailed, setHeroFailed] = useState(false);
  const showHero = Boolean(item.heroImageUrl) && !heroFailed;

  return (
    <div className={`browse-hero-media ${showHero ? 'browse-hero-media--image' : 'browse-hero-media--icon'}`}>
      {showHero ? (
        <img
          src={item.heroImageUrl!}
          className="browse-hero-media__image"
          loading="lazy"
          decoding="async"
          alt=""
          onError={() => setHeroFailed(true)}
        />
      ) : (
        <BrowseIcon iconUrl={item.iconUrl} name={item.name} className="browse-hero-media__fallback-icon" />
      )}
      {showHero && item.iconUrl && (
        <BrowseIcon iconUrl={item.iconUrl} name={item.name} className="browse-hero-media__identity-icon" />
      )}
    </div>
  );
}
