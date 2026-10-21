/** @type {import('postcss-load-config').Config} */
export default {
  // Tailwind 4 moved its PostCSS plugin to its own package and does vendor
  // prefixing itself, so autoprefixer is no longer listed.
  plugins: {
    '@tailwindcss/postcss': {},
  },
};
