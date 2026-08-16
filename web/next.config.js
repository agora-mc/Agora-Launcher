/** @type {import('next').NextConfig} */
const nextConfig = {
  output: 'export',
  distDir: 'dist',
  images: {
    unoptimized: true,
  },
  // The website mirrors the desktop app's Help & Guide from
  // desktop/src/data/guideContent.ts, which docs/README.md designates as the
  // source of truth for that copy. Importing it directly (rather than copying
  // it into web/) is what keeps the two from drifting apart.
  experimental: {
    externalDir: true,
  },
  async headers() {
    return [
      {
        source: '/:path*',
        headers: [
          {
            key: 'Content-Security-Policy',
            value: [
              "default-src 'self';",
              "img-src 'self' data: https: blob:;",
              "style-src 'self' 'unsafe-inline';",
              "script-src 'self';",
              "connect-src 'self' https://api.github.com https://api.modrinth.com https://*.modrinthcdn.com https://cdn.modrinth.com;",
              "font-src 'self' data:;",
              "object-src 'none';",
              "base-uri 'self';",
              "frame-ancestors 'none';",
              "form-action 'self';",
            ].join(' '),
          },
        ],
      },
    ];
  },
};

export default nextConfig;
