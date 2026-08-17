'use client';

import { useEffect, useRef } from 'react';
import { AmbienceEngine } from '@/features/ambience/engine/engine';

/**
 * LivingHero — the desktop living background, boxed into the homepage hero.
 *
 * Renders a `calm`-profile world (terrain, sky, weather, a few animals; no
 * eggs, no buddy, reduced particles) fitted to THIS container via the Stage 1
 * `getBounds` + `host` engine options. No audio: `musicOn` stays false (and
 * the engine's soundOn defaults to false), so the lazy tracks chunk is never
 * loaded.
 *
 * The engine touches `window` only inside `start()`/`attach()` (effects), so
 * this component is safe to render anywhere. The caller keeps the static
 * indigo hero behind it; under `prefers-reduced-motion` we do not mount at
 * all, and that fallback is all the visitor sees.
 */

function reducedMotionPref(): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia('(prefers-reduced-motion: reduce)').matches
    : false;
}

export default function LivingHero() {
  const hostRef = useRef<HTMLDivElement>(null);
  const bgRef = useRef<HTMLCanvasElement>(null);
  const fxRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const host = hostRef.current;
    const bg = bgRef.current;
    const fx = fxRef.current;
    if (!host || !bg || !fx) return;
    // Reduced-motion visitors keep the static hero: no canvas, no rAF.
    if (reducedMotionPref()) return;

    const engine = new AmbienceEngine(bg, fx, {
      profile: 'calm',
      musicOn: false,
      reducedMotion: false,
      host,
      // Fit the world to this box, in client coordinates. getBoundingClientRect
      // is fresh per call, so the pointer math stays correct across scroll,
      // resize, and font-load layout shifts.
      getBounds: () => {
        const r = host.getBoundingClientRect();
        return { left: r.left, top: r.top, w: r.width, h: r.height };
      },
    });
    engine.start();
    return () => engine.stop();
  }, []);

  return (
    <div ref={hostRef} className="living-hero" aria-hidden="true">
      <canvas ref={bgRef} className="living-hero-bg" />
      <div className="living-hero-vig" />
      <canvas ref={fxRef} className="living-hero-fx" />
    </div>
  );
}
