import Link from 'next/link';
import { GITHUB_RELEASES_URL } from '@/lib/site';
import { getDocNav } from '@/lib/docs';
import { getGuideTopics } from '@/lib/guide';

export const metadata = {
  title: 'Documentation - Agora',
  description:
    'Find the right Agora documentation: install and first run, task guides, troubleshooting, CLI reference, and contributor references.',
};

const ROUTES = [
  {
    href: '/docs/install',
    heading: 'I am new to Agora',
    body: 'Download a package, work through first-run setup, and reach a launch you can recover from.',
    cta: 'Install and first run',
  },
  {
    href: '/docs/troubleshooting',
    heading: 'Something is not working',
    body: 'Start from the symptom: launch blocked, loader mismatch, missing Java, crash after launch, offline failures.',
    cta: 'Troubleshooting',
  },
  {
    href: '/docs/development',
    heading: 'I want to build or contribute',
    body: 'Local builds, validation gates, architecture boundaries, registry curation, and release procedure.',
    cta: 'Development guide',
  },
];

const WAYS_TO_READ = [
  {
    href: '/docs/tour',
    title: 'Show me',
    body: 'An annotated screenshot tour of the six screens that cover most of what Agora does.',
  },
  {
    href: '/docs/guides',
    title: 'Walk me through it',
    body: 'Numbered steps for a specific task, assuming no prior modding knowledge. Every guide has this version.',
  },
  {
    href: '/docs/guides/modding-foundations#advanced',
    title: 'Explain how it works',
    body: 'The same guides written a second time as models rather than recipes — for readers who would rather understand than follow.',
  },
  {
    href: '/docs/cli',
    title: 'Just give me the syntax',
    body: 'Exhaustive reference: commands, flags, output shapes, exit codes, file locations, and evidence collection.',
  },
];

const COMMON_QUESTIONS = [
  { href: '/docs/guides/getting-started', label: 'How do I set up my first instance?' },
  { href: '/docs/guides/modding-foundations', label: 'What are versions, loaders, and dependencies?' },
  { href: '/docs/guides/launching', label: 'Why is my launch blocked, and what is a health check?' },
  { href: '/docs/guides/crash-recovery', label: 'Minecraft crashed — how do I find the cause?' },
  { href: '/docs/guides/snapshots-loadouts', label: 'What does a snapshot actually protect?' },
  { href: '/docs/guides/privacy-offline', label: 'What does Agora send, and how do I play offline?' },
  { href: '/docs/guides/packs-sharing', label: 'How do I move a setup to another machine?' },
  { href: '/docs/guides/java-performance', label: 'How much memory should I give Minecraft?' },
];

export default async function DocsPage() {
  const sections = await getDocNav();
  const library = sections.filter((section) => section.audience !== 'internal');
  const guideCount = getGuideTopics().length;

  return (
    <div className="min-w-0 space-y-14">
      <header>
        <p className="text-sm font-semibold uppercase tracking-wide text-indigo-600 dark:text-indigo-400">
          Agora documentation
        </p>
        <h1 className="mt-2 text-4xl font-bold tracking-tight">What are you trying to do?</h1>
        <p className="mt-4 max-w-3xl text-lg leading-8 text-gray-600 dark:text-gray-300">
          Agora is documented in a few different ways on purpose, because
          &ldquo;show me a picture&rdquo;, &ldquo;give me the steps&rdquo;, &ldquo;explain the
          model&rdquo;, and &ldquo;just the flags&rdquo; are different questions. Pick the
          lane that fits, or jump straight to a topic below.
        </p>
      </header>

      <section>
        <div className="grid gap-4 md:grid-cols-3">
          {ROUTES.map((route) => (
            <Link
              key={route.href}
              href={route.href}
              className="flex flex-col rounded-xl border bg-white p-6 shadow-sm transition hover:border-indigo-400 dark:border-gray-700 dark:bg-gray-800"
            >
              <h2 className="text-lg font-bold">{route.heading}</h2>
              <p className="mt-2 flex-1 text-sm leading-6 text-gray-600 dark:text-gray-300">
                {route.body}
              </p>
              <span className="mt-4 font-semibold text-indigo-600 dark:text-indigo-400">
                {route.cta} →
              </span>
            </Link>
          ))}
        </div>
      </section>

      <section>
        <h2 className="text-2xl font-bold">Four ways to read the same thing</h2>
        <p className="mt-2 max-w-3xl text-gray-600 dark:text-gray-300">
          These are not four different sets of facts. They are the same product
          described at four levels of detail, so you can choose the one that
          makes sense to you and switch when it stops helping.
        </p>
        <div className="mt-5 grid gap-4 sm:grid-cols-2">
          {WAYS_TO_READ.map((way) => (
            <Link
              key={way.href}
              href={way.href}
              className="rounded-xl border p-5 transition hover:border-indigo-400 dark:border-gray-700"
            >
              <h3 className="font-semibold text-indigo-700 dark:text-indigo-400">{way.title}</h3>
              <p className="mt-2 text-sm leading-6 text-gray-600 dark:text-gray-300">{way.body}</p>
            </Link>
          ))}
        </div>
      </section>

      <section>
        <h2 className="text-2xl font-bold">Common questions</h2>
        <p className="mt-2 text-gray-600 dark:text-gray-300">
          Straight to the guide that answers it.
        </p>
        <ul className="mt-4 grid gap-2 sm:grid-cols-2">
          {COMMON_QUESTIONS.map((question) => (
            <li key={question.href}>
              <Link
                href={question.href}
                className="block rounded-lg border px-4 py-3 text-sm transition hover:border-indigo-400 hover:text-indigo-700 dark:border-gray-700 dark:hover:text-indigo-400"
              >
                {question.label}
              </Link>
            </li>
          ))}
        </ul>
        <Link
          href="/docs/guides"
          className="mt-4 inline-flex font-semibold text-indigo-600 hover:underline dark:text-indigo-400"
        >
          All {guideCount} task guides
        </Link>
      </section>

      <section className="rounded-xl border border-indigo-200 bg-indigo-50/60 p-6 dark:border-indigo-900 dark:bg-indigo-950/30">
        <h2 className="text-xl font-bold">Already have Agora installed?</h2>
        <p className="mt-2 max-w-3xl leading-7 text-gray-700 dark:text-gray-300">
          The app has the same guides built in under <strong>Help &amp; Guide</strong>,
          where they are searchable and link directly to the screens they
          describe. This website publishes them so you can read them before
          installing, or send someone a link to a specific answer.
        </p>
      </section>

      <section className="space-y-8">
        <div>
          <h2 className="text-2xl font-bold">Reference library</h2>
          <p className="mt-2 max-w-3xl text-gray-600 dark:text-gray-300">
            Rendered directly from the repository&rsquo;s markdown, so the site and
            the source cannot disagree. Grouped by who needs them.
          </p>
        </div>
        {library.map((section) => (
          <div key={section.audience}>
            <h3 className="text-lg font-semibold">{section.label}</h3>
            <div className="mt-4 space-y-5">
              {section.groups.map((group) => (
                <div key={group.group}>
                  <p className="text-sm font-medium text-gray-500 dark:text-gray-400">
                    {group.group}
                  </p>
                  <div className="mt-2 grid gap-3 sm:grid-cols-2">
                    {group.docs.map((doc) => (
                      <Link
                        key={doc.slug}
                        href={`/docs/${doc.slug}`}
                        className="rounded-xl border bg-white p-4 shadow-sm transition hover:border-indigo-400 dark:border-gray-700 dark:bg-gray-800"
                      >
                        <h4 className="font-semibold text-indigo-700 dark:text-indigo-400">
                          {doc.title}
                        </h4>
                        {doc.description && (
                          <p className="mt-1 text-sm leading-6 text-gray-600 dark:text-gray-300">
                            {doc.description}
                          </p>
                        )}
                      </Link>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </div>
        ))}
      </section>

      <section className="rounded-xl border bg-white p-6 dark:border-gray-700 dark:bg-gray-800">
        <h2 className="text-xl font-bold">Elsewhere</h2>
        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <a
            href={GITHUB_RELEASES_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="font-medium text-indigo-600 hover:underline dark:text-indigo-400"
          >
            Desktop downloads and release notes
          </a>
          <Link
            href="/about"
            className="font-medium text-indigo-600 hover:underline dark:text-indigo-400"
          >
            Why Agora exists and how it is funded
          </Link>
        </div>
        <p className="mt-4 text-sm text-gray-500 dark:text-gray-400">
          Desktop packages ship in <code>v*</code> releases. <code>registry-*</code> releases
          contain catalog data assets, not installers.
        </p>
      </section>
    </div>
  );
}
