import type { Metadata } from 'next';
import './globals.css';
import { Shell } from '@/components/Shell';

export const metadata: Metadata = {
  title: {
    default: 'Agora Minecraft Mod Launcher',
    template: '%s · Agora',
  },
  description:
    'A community-curated Minecraft launcher with dependency-aware installs, recovery tools, direct or delegated launch, and an ad-free public catalog.',
};

/*
 * `className="dark"` is deliberate and permanent. The site follows the
 * desktop app's Civic Gold preset, which is a dark theme (`colorMode:
 * 'dark'`), so tailwind.config.js sets `darkMode: 'class'` and this hard-codes
 * it on — every `dark:` variant in the existing markup applies unconditionally
 * rather than following the visitor's OS preference.
 */
export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className="dark">
      <body>
        <Shell>{children}</Shell>
      </body>
    </html>
  );
}
