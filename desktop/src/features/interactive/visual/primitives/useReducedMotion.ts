import { useEffect, useState } from 'react';

/**
 * Reduced-motion detection for interactive visuals.
 *
 * Honors the app-wide preference applied by the ThemeProvider as a
 * `data-motion` attribute on the document root (`system | reduced | full`),
 * falling back to the OS `prefers-reduced-motion` media query.
 *
 * This lives in `visual/` primitives so shared components never need an
 * app-layer import to honor motion preferences.
 */

function detectReducedMotion(): boolean {
  if (typeof window === 'undefined') return false;
  const attr = document.documentElement.getAttribute('data-motion');
  if (attr === 'reduced') return true;
  if (attr === 'full') return false;
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

export function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState<boolean>(detectReducedMotion);

  useEffect(() => {
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
    const update = () => setReduced(detectReducedMotion());
    update();
    mq.addEventListener('change', update);
    const observer = new MutationObserver(update);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-motion'],
    });
    return () => {
      mq.removeEventListener('change', update);
      observer.disconnect();
    };
  }, []);

  return reduced;
}
