import {
  Info,
  Sparkles,
  SlidersHorizontal,
  Vote,
  Code2,
  HandCoins,
  Eye,
  HeartHandshake,
  ShieldCheck,
  GitBranch,
  MessagesSquare,
} from 'lucide-react';
import { agoraRepositoryUrl, agoraDiscordUrl, agoraSponsorsUrl } from '../lib/brandConfig';

const PILLARS = [
  {
    icon: SlidersHorizontal,
    title: 'Customizable to you',
    body: 'Theme every corner of Agora — fonts, colors, density, corners, motion, and more. If it does not feel like yours, keep tuning until it does.',
  },
  {
    icon: Vote,
    title: 'Democratic community voting',
    body: 'The community votes on what belongs in the curated list. No corporate editors, no pay-to-play — the players decide what is worth keeping.',
  },
  {
    icon: Code2,
    title: 'Open source',
    body: 'Every line of the launcher, the compiler, and the registry is public. Anyone can read it, audit it, and contribute to it.',
  },
  {
    icon: HandCoins,
    title: 'Free and ad-free',
    body: 'Agora costs nothing and shows no ads. There is no data-mining business model hiding behind the launcher — it exists to serve the community.',
  },
  {
    icon: Eye,
    title: 'Transparent',
    body: 'The registry, the votes, the review history, and the moderation log are all public. What changed, who voted, and why is never hidden.',
  },
  {
    icon: HeartHandshake,
    title: 'Donations, not for-profit',
    body: 'Agora is funded by the people who use it — through donations that keep the lights on — not by selling your attention or your data. If you enjoy Agora, please consider sponsoring to keep it free and to help fund awesome new projects in the future.',
  },
  {
    icon: ShieldCheck,
    title: 'Autonomous and decentralized',
    body: 'Agora does not depend on a corporate backend. Data ships through GitHub Release Assets and static files, so the platform keeps working on your terms.',
  },
];

export function About() {
  return (
    <div className="space-y-6">
      {/* Header */}
      <section className="relative overflow-hidden agora-hero">
        <div className="absolute -right-16 -top-20 h-56 w-56 rounded-full border border-white/10" aria-hidden="true" />
        <div className="absolute -bottom-28 right-20 h-64 w-64 rounded-full border border-white/10" aria-hidden="true" />
        <div className="relative">
          <div className="mb-3 flex items-center gap-3">
            <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-white/10 ring-1 ring-white/20">
              <Info className="h-6 w-6" aria-hidden="true" />
            </div>
            <h2 className="text-2xl font-bold sm:text-3xl">The Agora Difference</h2>
          </div>
          <p className="max-w-2xl text-sm leading-6 text-white/80 sm:text-base">
            Agora is a bespoke boutique, not a warehouse. Every entry in the catalog is curated,
            tailored, and made accessible for you — a hand-picked selection of the best Minecraft
            has to offer, free of ads, tracking, and bloat.
          </p>
        </div>
      </section>

      {/* What makes Agora unique */}
      <section className="rounded-xl border border-border bg-card p-6">
        <div className="flex items-center gap-2 mb-3">
          <Sparkles className="h-5 w-5 text-primary" aria-hidden="true" />
          <h3 className="text-lg font-semibold">What makes Agora unique</h3>
        </div>
        <p className="text-sm leading-6 text-muted-foreground">
          Most launchers and mod platforms are run for profit, are littered with ads, or bury
          great mods under a mountain of mediocre downloads. Agora is the opposite — it puts the
          community in charge and keeps the experience personal, private, and free.
        </p>
      </section>

      {/* Pillars */}
      <div className="grid gap-4 sm:grid-cols-2">
        {PILLARS.map((pillar) => (
          <div key={pillar.title} className="rounded-xl border border-border bg-card p-5">
            <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10 text-primary ring-1 ring-primary/20 mb-3">
              <pillar.icon className="h-5 w-5" aria-hidden="true" />
            </div>
            <h4 className="font-semibold mb-1">{pillar.title}</h4>
            <p className="text-sm leading-5 text-muted-foreground">{pillar.body}</p>
            {pillar.title === 'Donations, not for-profit' && (
              <a
                href={agoraSponsorsUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="mt-3 inline-flex text-sm font-medium text-pink-600 hover:underline dark:text-pink-400"
              >
                ♥ Sponsor on GitHub — support Agora’s future
              </a>
            )}
          </div>
        ))}
      </div>

      {/* Tailored & accessible */}
      <section className="rounded-xl border border-border bg-card p-6">
        <h3 className="text-lg font-semibold mb-2">Tailored and accessible for you</h3>
        <p className="text-sm leading-6 text-muted-foreground">
          Agora is built for the person in front of the screen. Interface scaling, high-contrast
          themes, reduced motion, screen-reader-friendly labels, and a curated catalog that gets
          out of your way — all of it is designed so the launcher works the way you need it to,
          not the way a marketing team wants you to use it.
        </p>
      </section>

      {/* Support Agora */}
      <section className="rounded-xl border border-pink-500/20 bg-pink-500/10 p-6">
        <div className="flex items-center gap-2 mb-3">
          <HeartHandshake className="h-5 w-5 text-pink-600 dark:text-pink-400" aria-hidden="true" />
          <h3 className="text-lg font-semibold">Support Agora’s future</h3>
        </div>
        <p className="text-sm leading-6 text-muted-foreground mb-4">
          Agora is free, open source, and ad-free — built by a solo developer who loves the Minecraft community. If Agora has made modding easier or more enjoyable for you, please consider sponsoring its development. Your donation keeps Agora improving and helps fund awesome new projects for the community. Every contribution means a lot — thank you!
        </p>
        <a
          href={agoraSponsorsUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center gap-2 rounded-lg bg-pink-600 px-5 py-2.5 text-sm font-semibold text-white hover:bg-pink-700"
        >
          <HeartHandshake className="h-4 w-4" aria-hidden="true" />
          Sponsor on GitHub
        </a>
      </section>

      {/* Get involved */}
      <section className="agora-panel-accent rounded-xl border border-primary/25 p-6">
        <div className="flex items-center gap-2 mb-3">
          <HeartHandshake className="h-5 w-5 text-primary" aria-hidden="true" />
          <h3 className="text-lg font-semibold">Get involved</h3>
        </div>
        <p className="text-sm leading-6 text-muted-foreground mb-4">
          Agora is a community project. Browse the source, review and vote on entries, join the
          conversation, or support the project with a donation so it can stay free and ad-free for
          everyone.
        </p>
        <div className="flex flex-wrap gap-3">
          {agoraRepositoryUrl && (
            <a
              href={agoraRepositoryUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-2 rounded-lg border border-border px-4 py-2 text-sm font-semibold hover:bg-accent"
            >
              <GitBranch className="h-4 w-4" aria-hidden="true" />
              GitHub Repository
            </a>
          )}
          {agoraDiscordUrl && (
            <a
              href={agoraDiscordUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-2 rounded-lg bg-[#5865F2] px-4 py-2 text-sm font-semibold text-white hover:bg-[#4752c4]"
            >
              <MessagesSquare className="h-4 w-4" aria-hidden="true" />
              Join the Discord
            </a>
          )}
          <a
            href={agoraSponsorsUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-2 rounded-lg bg-pink-600 px-4 py-2 text-sm font-semibold text-white hover:bg-pink-700"
          >
            <HeartHandshake className="h-4 w-4" aria-hidden="true" />
            Donate
          </a>
        </div>
      </section>
    </div>
  );
}
