'use client';

import dynamic from 'next/dynamic';

/**
 * HeroDiorama — client-only mount of the living hero.
 *
 * `next/dynamic` with `ssr: false` is only legal inside a Client Component.
 * The homepage is a Server Component, so this thin wrapper exists to keep the
 * world's window-touching engine out of the static-export prerender: at build
 * time the hero renders as nothing (the page's indigo fallback shows), and
 * only after hydration does LivingHero construct its canvases and start the
 * frame loop.
 */

const LivingHero = dynamic(() => import('@/components/LivingHero'), { ssr: false });

export default function HeroDiorama() {
  return <LivingHero />;
}
