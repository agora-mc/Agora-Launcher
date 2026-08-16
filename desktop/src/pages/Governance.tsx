import {
  Landmark,
  Scale,
  ThumbsUp,
  GitBranch,
  MessagesSquare,
  ShieldCheck,
  BookMarked,
  ArrowUp,
  ArrowDown,
} from 'lucide-react';
import { agoraRepositoryUrl, agoraDiscordUrl } from '../lib/brandConfig';

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

export function Governance() {
  return (
    <div className="space-y-6">
      {/* Header */}
      <section className="relative overflow-hidden agora-hero">
        <div className="absolute -right-16 -top-20 h-56 w-56 rounded-full border border-white/10" aria-hidden="true" />
        <div className="absolute -bottom-28 right-20 h-64 w-64 rounded-full border border-white/10" aria-hidden="true" />
        <div className="relative">
          <div className="mb-3 flex items-center gap-3">
            <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-white/10 ring-1 ring-white/20">
              <Landmark className="h-6 w-6" aria-hidden="true" />
            </div>
            <div>
              <h2 className="text-2xl font-bold sm:text-3xl">Community Governance</h2>
              <p className="text-white/65">
                Your votes shape the curated list — by quality, not by download count.
              </p>
            </div>
          </div>
          <p className="max-w-3xl text-sm leading-6 text-white/80 sm:text-base">
            Agora is a <strong className="text-white">community-curated</strong> platform. Every
            entry in the registry is reviewed and voted on by people like you, and the community’s
            scores decide what stays, what is promoted, and what is removed. Voting is how we keep
            the catalog a boutique selection of genuinely good content instead of a warehouse of
            whatever happens to be popular.
          </p>
        </div>
      </section>

      {/* Why vote */}
      <section className="rounded-xl border border-border bg-card p-6">
        <h3 className="text-xl font-semibold mb-2">Why your vote matters</h3>
        <p className="text-sm leading-6 text-muted-foreground sm:text-base">
          Your vote is a small, honest signal that compounds with everyone else’s. A mod that works
          flawlessly for a niche audience deserves its place even if it has few downloads; a
          bloated or abandoned project should not stay on top just because it was downloaded a lot
          in the past. When you vote, you help the community:
        </p>
        <ul className="mt-3 space-y-3 text-sm sm:text-base">
          <li className="flex items-start gap-2">
            <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
            <span><strong>Refine the curated list</strong> — surface the best mods, packs, shaders, and tools for everyone.</span>
          </li>
          <li className="flex items-start gap-2">
            <Scale className="mt-0.5 h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
            <span><strong>Rank by quality</strong>, not by the quantity of downloads — a list you can trust at a glance.</span>
          </li>
          <li className="flex items-start gap-2">
            <ThumbsUp className="mt-0.5 h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
            <span><strong>Reward great creators</strong> and protect players from broken, abandoned, or unsafe content.</span>
          </li>
        </ul>
        <p className="mt-4 rounded-lg bg-primary/10 px-4 py-3 text-sm text-foreground sm:text-base">
          Vote early and vote often — a few seconds of your time keeps the registry honest for
          everyone.
        </p>
      </section>

      {/* How to vote */}
      <section className="rounded-xl border border-border bg-card p-6">
        <h3 className="text-xl font-semibold mb-2">How to vote</h3>
        <p className="text-sm leading-6 text-muted-foreground mb-4 sm:text-base">
          You can vote two ways — in the Agora app itself, or directly on GitHub. Both feed the
          same score.
        </p>

        <div className="grid gap-4 sm:grid-cols-2">
          <div className="rounded-lg border border-border bg-muted/40 p-4">
            <div className="flex items-center gap-2 font-semibold text-sm mb-2">
              <ArrowUp className="h-4 w-4 text-primary" aria-hidden="true" />
              <ArrowDown className="h-4 w-4 text-primary" aria-hidden="true" />
              In the Agora app
            </div>
            <ol className="space-y-2.5 text-sm text-muted-foreground list-decimal list-inside sm:text-base">
              {VOTE_STEPS_IN_APP.map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ol>
          </div>

          <div className="rounded-lg border border-border bg-muted/40 p-4">
            <div className="flex items-center gap-2 font-semibold text-sm mb-2">
              <GitBranch className="h-4 w-4 text-primary" aria-hidden="true" />
              On GitHub
            </div>
            <ol className="space-y-2.5 text-sm text-muted-foreground list-decimal list-inside sm:text-base">
              {VOTE_STEPS_GITHUB.map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ol>
          </div>
        </div>
      </section>

      {/* Community moderation & voting */}
      <section className="rounded-xl border border-border bg-card p-6">
        <h3 className="text-xl font-semibold mb-2">How moderation &amp; voting work</h3>
        <div className="space-y-3 text-sm leading-6 text-muted-foreground sm:text-base">
          <p>
            Every entry in the registry is a flat, public manifest in the Agora repository. A
            nightly compiler turns those manifests into a signed database that your launcher
            downloads and verifies. The same pipeline reads the community’s votes, scores every
            item, and publishes a transparent record of what changed and why.
          </p>
          <p>
            <strong className="text-foreground">Reviews</strong> are structured, technical
            evaluations submitted through the community-review process. <strong className="text-foreground">Votes</strong> are
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
      <section className="rounded-xl border border-border bg-card p-6">
        <div className="flex items-center gap-2 mb-3">
          <BookMarked className="h-5 w-5 text-primary" aria-hidden="true" />
          <h3 className="text-xl font-semibold">Agora’s rules of engagement</h3>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          {RULES.map((rule) => (
            <div key={rule.title} className="rounded-lg border border-border bg-muted/40 p-4">
              <h4 className="font-semibold text-base mb-1">{rule.title}</h4>
              <p className="text-sm leading-6 text-muted-foreground">{rule.body}</p>
            </div>
          ))}
        </div>
        <p className="mt-4 text-sm text-muted-foreground">
          Want to socialize, share memes, or debate off-topic things? That belongs in the community
          spaces, not the registry. Join us on Discord below.
        </p>
      </section>

      {/* Community links */}
      <section className="agora-panel-accent rounded-xl border border-primary/25 p-6">
        <div className="flex items-center gap-2 mb-3">
          <MessagesSquare className="h-5 w-5 text-primary" aria-hidden="true" />
          <h3 className="text-xl font-semibold">Join the community</h3>
        </div>
        <p className="text-sm leading-6 text-muted-foreground mb-4 sm:text-base">
          Discuss mods, ask for help, and shape Agora’s direction with other players and curators.
        </p>
        <div className="flex flex-wrap gap-3">
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
        </div>
      </section>
    </div>
  );
}
