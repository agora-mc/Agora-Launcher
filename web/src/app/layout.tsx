import type { Metadata } from 'next';
import './globals.css';
import { Shell } from '@/components/Shell';

export const metadata: Metadata = {
  title: 'Agora Minecraft Mod Launcher',
  description:
    'A community-curated Minecraft launcher with dependency-aware installs, recovery tools, direct or delegated launch, and an ad-free public catalog.',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>
        <Shell>{children}</Shell>
      </body>
    </html>
  );
}
