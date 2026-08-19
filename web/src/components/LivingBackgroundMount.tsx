'use client';

import dynamic from 'next/dynamic';

/**
 * LivingBackgroundMount — client-only mount of the living world.
 *
 * `next/dynamic` with `ssr: false` is only legal inside a Client Component,
 * and the Shell is a Server Component, so this thin wrapper exists to keep
 * the world's window-touching engine out of the static-export prerender. At
 * build time it renders nothing (the flat civic background shows); only after
 * hydration does LivingBackground construct its canvases and start the loop.
 */

const LivingBackground = dynamic(() => import('@/components/LivingBackground'), { ssr: false });

export default function LivingBackgroundMount() {
  return <LivingBackground />;
}
