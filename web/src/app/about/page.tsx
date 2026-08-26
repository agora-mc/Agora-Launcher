import Link from 'next/link';
import { GITHUB_REPO_URL, DISCORD_URL, SPONSORS_URL } from '@/lib/site';

const PILLARS = [
  {
    title: 'Customizable to you',
    body: 'Theme every corner of Agora — fonts, colors, density, corners, motion, and more. If it does not feel like yours, keep tuning until it does.',
  },
  {
    title: 'Democratic community voting',
    body: 'The community votes on what belongs in the curated list. No corporate editors, no pay-to-play — the players decide what is worth keeping.',
  },
  {
    title: 'Open source',
    body: 'Every line of the launcher, the compiler, and the registry is public. Anyone can read it, audit it, and contribute to it.',
  },
  {
    title: 'Free and ad-free',
    body: 'Agora costs nothing and shows no ads. There is no data-mining business model hiding behind the launcher — it exists to serve the community.',
  },
  {
    title: 'Transparent',
    body: 'The registry, the votes, the review history, and the moderation log are all public. What changed, who voted, and why is never hidden.',
  },
  {
    title: 'Donations, not for-profit',
    body: 'Agora is funded by the people who use it — through donations that keep the lights on — not by selling your attention or your data. If Agora has made your modding easier or more enjoyable, please consider sponsoring to keep it free and to support awesome new projects in the future.',
  },
  {
    title: 'Autonomous and decentralized',
    body: 'Agora does not depend on a corporate backend. Data ships through GitHub Release Assets and static files, so the platform keeps working on your terms.',
  },
];

export const metadata = {
  title: 'The Agora Difference',
};

export default function AboutPage() {
  return (
    <div className="page-read space-y-8">
      <header className="panel text-center">
        <p className="eyebrow">Why Agora</p>
        <h1 className="mt-2.5 bg-gold-sheen bg-clip-text text-4xl font-extrabold tracking-tight text-transparent">
          The Agora Difference
        </h1>
        <p className="mx-auto mt-3 max-w-xl text-lg text-ink-muted">
          A different kind of Minecraft mod platform.
        </p>
      </header>

      <section className="panel border-gold/35 bg-[linear-gradient(135deg,rgba(194,139,40,0.15),rgba(23,38,59,0.93)_55%)]">
        <p className="text-lg font-medium leading-7">
          Agora is a bespoke boutique, not a warehouse. Every entry in the catalog is curated,
          tailored, and made accessible for you — a hand-picked selection of the best Minecraft
          has to offer, free of ads, tracking, and bloat.
        </p>
      </section>

      <section className="panel space-y-4">
        <h2 className="rule-gold text-xl font-semibold text-ink">Why "Agora"?</h2>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          The name "Agora" comes from the ancient Greek word for a public gathering place. Just as the Agora was the heart of social and political life in ancient Athens, Agora is designed to be a central hub for the Minecraft modding community.
        </p>
      </section>

      <section className="panel space-y-4">
        <h2 className="rule-gold text-xl font-semibold text-ink">What makes Agora unique</h2>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          Most launchers and mod platforms are run for profit, alienate non-tech savvy and visual focused users, are littered with ads, and/or bury
          great mods under a mountain of mediocre downloads. Agora is the opposite — it puts the
          community in charge and keeps the experience personal, private, and free.
          Too much of the modding community it built around which platform is used and their philosophies. Modding was never meant to be an industry, it was meant to be a community. As with the Origin of the word in ancient Greece Agora is the beating heart that brings that community together.
        </p>
      </section>

      <section className="panel space-y-4">
        <h2 className="rule-gold text-xl font-semibold text-ink">The pillars</h2>
        <ul className="grid gap-4 sm:grid-cols-2">
          {PILLARS.map((pillar) => (
            <li key={pillar.title} className="panel-inset p-5">
              <h3 className="font-semibold text-indigo-700 dark:text-indigo-400">{pillar.title}</h3>
              <p className="mt-1 text-sm leading-6 text-gray-600 dark:text-gray-400">{pillar.body}</p>
              {pillar.title === 'Donations, not for-profit' && (
                <a
                  href={SPONSORS_URL}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="mt-2 inline-flex text-sm font-medium text-pink-600 hover:underline dark:text-pink-400"
                >
                  ♥ Sponsor on GitHub — support Agora’s future
                </a>
              )}
            </li>
          ))}
        </ul>
      </section>

      <section className="panel space-y-4">
        <h2 className="rule-gold text-xl font-semibold text-ink">Tailored and accessible for you</h2>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          Agora is built for the person in front of the screen. Interface scaling, high-contrast
          themes, reduced motion, screen-reader-friendly labels, and a curated catalog that gets
          out of your way — all of it is designed so the launcher works the way you need it to,
          not the way a marketing team wants you to use it.
        </p>
      </section>

      <section className="panel space-y-4">
        <h2 className="rule-gold text-xl font-semibold text-ink">Get involved</h2>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          Agora is a community project. Browse the source, review and vote on entries, join the
          conversation, or support the project with a donation so it can stay free and ad-free for
          everyone. You can also{' '}
          <Link href="/docs" className="font-medium text-indigo-600 hover:underline dark:text-indigo-400">
            read the docs
          </Link>{' '}
          to learn how it all fits together.
        </p>
        <div className="panel-inset border-pink-700/60 bg-[linear-gradient(135deg,rgba(184,69,106,0.16),rgba(29,47,71,0.86)_55%)] p-5">
          <h3 className="font-semibold text-pink-800 dark:text-pink-200">Support Agora’s future</h3>
          <p className="mt-1 text-sm leading-6 text-pink-700 dark:text-pink-300">
            Agora is built and maintained by a solo developer who cares deeply about the Minecraft community. If Agora saves you time, helps you discover great mods, or just makes modding more fun — please consider sponsoring. Your donation keeps Agora free, ad-free, and improving, and it helps fund awesome new projects down the road. Every contribution, big or small, means a lot — thank you!
          </p>
          <a
            href={SPONSORS_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="mt-3 inline-flex items-center gap-2 rounded-lg bg-pink-600 px-4 py-2 text-sm font-semibold text-white hover:bg-pink-700"
          >
            <span aria-hidden="true">♥</span> Sponsor on GitHub
          </a>
        </div>
        <div className="flex flex-wrap gap-3 pt-1">
          <a
            href={GITHUB_REPO_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="rounded-lg border border-gray-300 px-4 py-2 text-sm font-semibold text-gray-800 hover:border-indigo-400 hover:text-indigo-600 dark:border-gray-600 dark:text-gray-200 dark:hover:border-indigo-400 dark:hover:text-indigo-400"
          >
            GitHub Repository
          </a>
          <a
            href={DISCORD_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="rounded-lg bg-[#5865F2] px-4 py-2 text-sm font-semibold text-white hover:bg-[#4752c4]"
          >
            Join the Discord
          </a>
        </div>
      </section>
    </div>
  );
}
