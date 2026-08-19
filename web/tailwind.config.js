/** @type {import('tailwindcss').Config} */

/*
 * Civic Gold — the web palette, ported from the desktop app's default
 * appearance preset (`DEFAULT_UI_PREFERENCES` in
 * desktop/src/components/theme/theme-provider.tsx):
 *
 *   background #091321   surface #17263b   nav #0d1929
 *   accent     #c28b28   text    #f4ead4   muted text #e8d9bb
 *
 * The site is DARK-ONLY (the desktop preset is `colorMode: 'dark'`), which is
 * why `darkMode: 'class'` is paired with a hard-coded `class="dark"` on <html>
 * in app/layout.tsx. Every `dark:` variant therefore applies unconditionally.
 *
 * Rather than rewrite several hundred utility classes across every page, the
 * `gray` and `indigo` scales are REDEFINED here:
 *
 *   gray-*   navy at the dark end, warm cream at the light end. So the
 *            existing `bg-gray-900` / `dark:bg-gray-800` / `text-gray-300`
 *            vocabulary lands on the civic palette without being touched.
 *   indigo-* the civic gold ramp, so every existing accent becomes gold.
 *   pink-*   warmed toward rose so the sponsor accents sit beside gold
 *            instead of fighting it.
 *   amber-*  bronzed, so notices read as a sibling of gold, not a clash.
 *
 * Semantic aliases (canvas/surface/nav/gold/ink/hairline) are the preferred
 * names for NEW markup; the remapped scales exist to carry the old markup.
 */

const CIVIC = {
  canvas: '#091321',
  canvasDeep: '#060d18',
  nav: '#0d1929',
  surface: '#17263b',
  surfaceRaised: '#1d2f47',
  hairline: '#24374f',
  gold: '#c28b28',
  goldBright: '#e0b354',
  ink: '#f4ead4',
  inkMuted: '#e8d9bb',
};

export default {
  darkMode: 'class',
  content: [
    './src/pages/**/*.{js,ts,jsx,tsx,mdx}',
    './src/components/**/*.{js,ts,jsx,tsx,mdx}',
    './src/app/**/*.{js,ts,jsx,tsx,mdx}',
  ],
  theme: {
    extend: {
      colors: {
        // ── semantic names (prefer these in new markup) ──
        canvas: CIVIC.canvas,
        'canvas-deep': CIVIC.canvasDeep,
        nav: CIVIC.nav,
        surface: CIVIC.surface,
        'surface-raised': CIVIC.surfaceRaised,
        hairline: CIVIC.hairline,
        gold: CIVIC.gold,
        'gold-bright': CIVIC.goldBright,
        ink: CIVIC.ink,
        'ink-muted': CIVIC.inkMuted,

        // ── remapped scales: carry the pre-existing markup onto the palette ──
        gray: {
          50: '#faf4e6',
          100: CIVIC.ink,
          200: '#f0e4c9',
          300: CIVIC.inkMuted,
          400: '#c3b596',
          500: '#8fa3ba',
          600: '#5d738f',
          700: CIVIC.hairline,
          800: CIVIC.surface,
          900: CIVIC.canvas,
          950: CIVIC.canvasDeep,
        },
        indigo: {
          50: '#fcf6e8',
          100: '#f8ecd0',
          200: '#f2dba6',
          300: '#ebc77a',
          400: CIVIC.goldBright,
          500: '#d4a03e',
          600: CIVIC.gold,
          700: '#a2731f',
          800: '#5c4211',
          900: '#3d2c0c',
          950: '#2a1e08',
        },
        pink: {
          50: '#fdf2f4',
          100: '#fbe4e9',
          200: '#f5c3ce',
          300: '#e99bad',
          400: '#d97a92',
          500: '#c85c78',
          600: '#b8456a',
          700: '#963656',
          800: '#5e2038',
          900: '#3f1626',
          950: '#2a0e19',
        },
        amber: {
          50: '#fbf1dc',
          100: '#f6e3bb',
          200: '#eccb8b',
          300: '#dfae5c',
          400: '#cf9538',
          500: '#b87d24',
          600: '#9a651c',
          700: '#7a4f16',
          800: '#4a3110',
          900: '#33220c',
          950: '#221708',
        },
        sky: {
          100: '#dceaf5',
          200: '#b9d5eb',
          300: '#8ab8db',
          400: '#5c99c6',
          500: '#3d7bab',
          600: '#2f6089',
          700: '#254c6c',
          800: '#1b3750',
          900: '#132639',
        },
      },
      // Bare `border` uses borderColor.DEFAULT, which Tailwind points at
      // gray-200 — now a bright cream in this palette. Repoint it at the
      // hairline so an uncoloured `border` reads correctly on navy.
      borderColor: {
        DEFAULT: CIVIC.hairline,
      },
      opacity: {
        8: '0.08',
        12: '0.12',
        15: '0.15',
        18: '0.18',
        35: '0.35',
        65: '0.65',
        85: '0.85',
      },
      fontFamily: {
        display: ['var(--font-display)'],
        body: ['var(--font-body)'],
      },
      borderRadius: {
        lg: '0.75rem',
        xl: '0.9rem',
        '2xl': '1.15rem',
      },
      boxShadow: {
        // Gold-tinted depth: the palette has no neutral shadow that reads on navy.
        sm: '0 1px 2px rgba(3, 8, 15, 0.55)',
        DEFAULT: '0 2px 8px rgba(3, 8, 15, 0.5)',
        md: '0 6px 18px -6px rgba(3, 8, 15, 0.7)',
        lg: '0 14px 34px -12px rgba(3, 8, 15, 0.75)',
        glow: '0 0 0 1px rgba(194, 139, 40, 0.35), 0 10px 30px -12px rgba(194, 139, 40, 0.45)',
      },
      backgroundImage: {
        'gold-rule': 'linear-gradient(90deg, #c28b28, rgba(194, 139, 40, 0))',
        'gold-sheen': 'linear-gradient(135deg, #e0b354 0%, #c28b28 45%, #a2731f 100%)',
      },
    },
  },
  plugins: [],
};
