'use client';

import { useEffect, useRef } from 'react';
import { AmbienceEngine } from '@/features/ambience/engine/engine';

/**
 * LivingBackground — the desktop's living world, behind the whole site.
 *
 * Mounted once in the Shell, above the page switch. Deliberately constructed
 * WITHOUT the engine's `getBounds` / `host` options: omitting them is what
 * selects the original full-viewport behaviour (window-sized world, window
 * pointer tracking, document-level clicks), which is exactly what a site-wide
 * background wants. The boxed hero diorama that used to live here supplied
 * both; that is the only difference between the two modes.
 *
 * Profile is `calm`: terrain, sky, weather and a few animals — no eggs, no
 * buddy, and particles forced off (engine `setProfile`). No audio either:
 * `musicOn` stays false and `soundOn` defaults false, so the lazy tracks
 * chunk is never fetched. A website should not make noise.
 *
 * Legibility is not this component's job. The world renders at full strength
 * and the page's `.panel` boxes sit over it (globals.css), so text keeps an
 * opaque backing while the world stays visible in the gutters.
 */

function reducedMotionPref(): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia('(prefers-reduced-motion: reduce)').matches
    : false;
}

export default function LivingBackground() {
  const bgRef = useRef<HTMLCanvasElement>(null);
  const fxRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const bg = bgRef.current;
    const fx = fxRef.current;
    if (!bg || !fx) return;
    // Reduced-motion visitors keep the flat civic background: no canvas, no rAF.
    if (reducedMotionPref()) return;

    const engine = new AmbienceEngine(bg, fx, {
      profile: 'calm',
      musicOn: false,
      reducedMotion: false,
    });
    engine.start();
    return () => engine.stop();
  }, []);

  return (
    <div aria-hidden="true">
      <canvas ref={bgRef} className="ambience-canvas" />
      <div className="ambience-vig" />
      <canvas ref={fxRef} className="ambience-fx" />
    </div>
  );
}
