import Link from 'next/link';
import { GITHUB_REPO_URL, DISCORD_URL, SPONSORS_URL } from '@/lib/site';

const RULES = [
  {
    title: 'Stay technical',
    body: 'Comments and votes exist to evaluate the technical performance, stability, features, or usability of the asset in question — nothing else.',
  },
  {
    title: 'No noise',
    body: 'No memes, no off-topic banter, no update-begging ("1.21 when?"), and no philosophical debates in review threads.',
  },
  {
    title: 'Leave drama at the door',
    body: 'No cultural, political, or social drama. This platform is a curated asset repository, not a social media feed.',
  },
  {
    title: 'Be respectful',
    body: 'No aggression, entitlement, or personal attacks against mod creators, curators, or anyone else. Everyone here is a volunteer.',
  },
  {
    title: 'Zero tolerance',
    body: 'Violations result in immediate and permanent removal from the registry review system. Keep it kind and keep it technical.',
  },
];

const VOTE_STEPS_IN_APP = [
  'Open Settings → Accounts and sign in with GitHub. Your vote is public and tied to your GitHub identity.',
  'Browse to any curated item and open its details page.',
  'Use the ▲ / ▼ buttons next to the score to upvote or downvote the entry.',
  'Your vote is recorded immediately. You can change it at any time.',
];

const VOTE_STEPS_GITHUB = [
  'Open the item’s canonical vote issue on GitHub — it carries the registry-vote label.',
  'React with +1 (👍) to upvote or -1 (👎) to downvote.',
  'Only direct reactions on the canonical vote issue count. Reviews and comments never affect the score.',
];

export const metadata = {
  title: 'Community Governance — Agora',
  description:
    'How community voting and moderation work in Agora — votes, reviews, quarantine, and rules of engagement.',
};

export default function GovernancePage() {
  return (
    <div className="mx-auto max-w-4xl space-y-8">
      {/* Header */}
      <section className="relative overflow-hidden rounded-2xl bg-indigo-600 px-8 py-10 text-white dark:bg-indigo-700">
        <div className="pointer-events-none absolute -right-16 -top-20 h-56 w-56 rounded-full border border-white/10" aria-hidden="true" />
        <div className="pointer-events-none absolute -bottom-28 right-20 h-64 w-64 rounded-full border border-white/10" aria-hidden="true" />
        <div className="relative">
          <div className="mb-3 flex items-center gap-3">
            <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-white/10 ring-1 ring-white/20">
              <span aria-hidden="true" className="text-xl">◈</span>
            </div>
            <div>
              <h1 className="text-2xl font-bold sm:text-3xl">Community Governance</h1>
              <p className="text-indigo-100">Your votes shape the curated list — by quality, not by download count.</p>
            </div>
          </div>
          <p className="max-w-3xl text-sm leading-6 text-indigo-50 sm:text-base">
            Agora is a <strong className="text-white">community-curated</strong> platform. Every
            entry in the registry is reviewed and voted on by people like you, and the community’s
            scores decide what stays, what is promoted, and what is removed. Voting is how we keep
            the catalog a boutique selection of genuinely good content instead of a warehouse of
            whatever happens to be popular.
          </p>
        </div>
      </section>

      {/* Why vote */}
      <section className="rounded-xl border bg-white p-6 dark:border-gray-700 dark:bg-gray-800">
        <h2 className="mb-2 text-xl font-semibold">Why your vote matters</h2>
        <p className="text-sm leading-6 text-gray-600 dark:text-gray-400 sm:text-base">
          Your vote is a small, honest signal that compounds with everyone else’s. A mod that works
          flawlessly for a niche audience deserves its place even if it has few downloads; a
          bloated or abandoned project should not stay on top just because it was downloaded a lot
          in the past. When you vote, you help the community:
        </p>
        <ul className="mt-3 space-y-3 text-sm sm:text-base">
          <li className="flex items-start gap-2">
            <span className="mt-0.5 text-indigo-600 dark:text-indigo-400" aria-hidden="true">🛡</span>
            <span><strong>Refine the curated list</strong> — surface the best mods, packs, shaders, and tools for everyone.</span>
          </li>
          <li className="flex items-start gap-2">
            <span className="mt-0.5 text-indigo-600 dark:text-indigo-400" aria-hidden="true">⚖</span>
            <span><strong>Rank by quality</strong>, not by the quantity of downloads — a list you can trust at a glance.</span>
          </li>
          <li className="flex items-start gap-2">
            <span className="mt-0.5 text-indigo-600 dark:text-indigo-400" aria-hidden="true">👍</span>
            <span><strong>Reward great creators</strong> and protect players from broken, abandoned, or unsafe content.</span>
          </li>
        </ul>
        <p className="mt-4 rounded-lg bg-indigo-50 px-4 py-3 text-sm text-gray-900 dark:bg-indigo-950/40 dark:text-gray-100 sm:text-base">
          Vote early and vote often — a few seconds of your time keeps the registry honest for
          everyone.
        </p>
      </section>

      {/* How to vote */}
      <section className="rounded-xl border bg-white p-6 dark:border-gray-700 dark:bg-gray-800">
        <h2 className="mb-2 text-xl font-semibold">How to vote</h2>
        <p className="mb-4 text-sm leading-6 text-gray-600 dark:text-gray-400 sm:text-base">
          You can vote two ways — in the Agora app itself, or directly on GitHub. Both feed the
          same score.
        </p>

        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-lg border bg-gray-50 p-4 dark:border-gray-700 dark:bg-gray-800/50">
            <div className="mb-2 flex items-center gap-2 text-sm font-semibold">
              <span className="text-indigo-600 dark:text-indigo-400" aria-hidden="true">▲▼</span>
              In the Agora app
            </div>
            <ol className="list-inside list-decimal space-y-2.5 text-sm text-gray-600 dark:text-gray-400 sm:text-base">
              {VOTE_STEPS_IN_APP.map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ol>
          </div>

          <div className="rounded-lg border bg-gray-50 p-4 dark:border-gray-700 dark:bg-gray-800/50">
            <div className="mb-2 flex items-center gap-2 text-sm font-semibold">
              <span className="text-indigo-600 dark:text-indigo-400" aria-hidden="true">⎇</span>
              On GitHub
            </div>
            <ol className="list-inside list-decimal space-y-2.5 text-sm text-gray-600 dark:text-gray-400 sm:text-base">
              {VOTE_STEPS_GITHUB.map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ol>
          </div>
        </div>
      </section>

      {/* How moderation & voting work */}
      <section className="rounded-xl border bg-white p-6 dark:border-gray-700 dark:bg-gray-800">
        <h2 className="mb-2 text-xl font-semibold">How moderation &amp; voting work</h2>
        <div className="space-y-3 text-sm leading-6 text-gray-600 dark:text-gray-400 sm:text-base">
          <p>
            Every entry in the registry is a flat, public manifest in the Agora repository. A
            nightly compiler turns those manifests into a signed database that your launcher
            downloads and verifies. The same pipeline reads the community’s votes, scores every
            item, and publishes a transparent record of what changed and why.
          </p>
          <p>
            <strong className="text-gray-900 dark:text-gray-100">Reviews</strong> are structured, technical
            evaluations submitted through the community-review process. <strong className="text-gray-900 dark:text-gray-100">Votes</strong> are
            lightweight +1 / -1 signals that move an item’s score. The two work together: reviews
            explain <em>why</em> something is good or bad, and votes show how the wider community
            feels.
          </p>
          <p>
            Agora also watches for coordinated vote manipulation. Sudden, artificial bursts of
            votes are detected, quarantined, and reviewed by curators before they can distort the
            list. Curators make the final call, and everything is logged publicly — there is no
            hidden moderation.
          </p>
        </div>
      </section>

      {/* Rules */}
      <section className="rounded-xl border bg-white p-6 dark:border-gray-700 dark:bg-gray-800">
        <div className="mb-3 flex items-center gap-2">
          <span className="text-indigo-600 dark:text-indigo-400" aria-hidden="true">📖</span>
          <h2 className="text-xl font-semibold">Agora’s rules of engagement</h2>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          {RULES.map((rule) => (
            <div key={rule.title} className="rounded-lg border bg-gray-50 p-4 dark:border-gray-700 dark:bg-gray-800/50">
              <h3 className="mb-1 text-base font-semibold">{rule.title}</h3>
              <p className="text-sm leading-6 text-gray-600 dark:text-gray-400">{rule.body}</p>
            </div>
          ))}
        </div>
        <p className="mt-4 text-sm text-gray-500 dark:text-gray-400">
          Want to socialize, share memes, or debate off-topic things? That belongs in the community
          spaces, not the registry. Join us on Discord below.
        </p>
      </section>

      {/* Community links */}
      <section className="rounded-xl border border-indigo-200 bg-indigo-50/60 p-6 dark:border-indigo-900 dark:bg-indigo-950/30">
        <div className="mb-3 flex items-center gap-2">
          <span className="text-indigo-600 dark:text-indigo-400" aria-hidden="true">💬</span>
          <h2 className="text-xl font-semibold">Join the community</h2>
        </div>
        <p className="mb-4 text-sm leading-6 text-gray-600 dark:text-gray-400 sm:text-base">
          Discuss mods, ask for help, and shape Agora’s direction with other players and curators.
        </p>
        <div className="flex flex-wrap gap-3">
          <a
            href={DISCORD_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-2 rounded-lg bg-[#5865F2] px-4 py-2 text-sm font-semibold text-white hover:bg-[#4752c4]"
          >
            Join the Discord
          </a>
          <a
            href={GITHUB_REPO_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-2 rounded-lg border bg-white px-4 py-2 text-sm font-semibold hover:bg-gray-50 dark:border-gray-600 dark:bg-gray-800 dark:hover:bg-gray-700"
          >
            GitHub Repository
          </a>
          <a
            href={SPONSORS_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-2 rounded-lg bg-pink-600 px-4 py-2 text-sm font-semibold text-white hover:bg-pink-700"
          >
            <span aria-hidden="true">♥</span> Sponsor Agora
          </a>
        </div>
        
      </section>

      <section className="rounded-xl border bg-white p-6 dark:border-gray-700 dark:bg-gray-800">
        <h2 className="text-lg font-semibold">Learn more</h2>
        <div className="mt-3 flex flex-wrap gap-3">
          <Link href="/about" className="font-medium text-indigo-600 hover:underline dark:text-indigo-400">
            The Agora Difference →
          </Link>
          <Link href="/docs" className="font-medium text-indigo-600 hover:underline dark:text-indigo-400">
            Documentation →
          </Link>
        </div>
      </section>
    </div>
  );
}
