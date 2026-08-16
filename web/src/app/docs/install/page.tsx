import Link from 'next/link';
import { GITHUB_RELEASES_URL } from '@/lib/site';

export const metadata = {
  title: 'Install Agora — Agora Docs',
  description:
    'Download a packaged Agora release, complete first-run setup, and reach a recoverable first launch.',
};

const STEPS = [
  {
    title: 'Download the package for your operating system',
    body: 'Open the releases page and pick the installer or portable package that matches your platform from a published v* release.',
  },
  {
    title: 'Run onboarding and make the optional choices',
    body: 'Onboarding asks which optional services Agora may use. Every switch can be changed later in Settings, so nothing here is permanent.',
  },
  {
    title: 'Synchronize the signed registry',
    body: 'The registry is Agora’s curated catalog. Downloading it is what makes Browse and community governance data available.',
  },
  {
    title: 'Let Agora find or provision Java',
    body: 'Minecraft needs a Java runtime. Leave Java management on Automatic unless a pack tells you otherwise, and Agora will select or download a compatible one.',
  },
  {
    title: 'Create a small instance, or import a pack into a disposable one',
    body: 'An instance is one isolated Minecraft setup. Starting small keeps the first round of compatibility decisions manageable.',
  },
  {
    title: 'Review health findings, then play for at least 60 seconds',
    body: 'A session that lasts a minute is what lets Agora promote the setup to Last Known Good — the recovery point you will want later.',
  },
];

export default function InstallPage() {
  return (
    <div className="min-w-0 space-y-10">
      <header>
        <nav aria-label="Breadcrumb" className="mb-4 text-sm text-gray-500 dark:text-gray-400">
          <Link href="/docs" className="hover:text-indigo-600 dark:hover:text-indigo-400">
            Docs
          </Link>
          <span aria-hidden="true"> / </span>
          <span className="text-gray-700 dark:text-gray-300">Install Agora</span>
        </nav>
        <h1 className="text-3xl font-bold tracking-tight">Install Agora</h1>
        <p className="mt-3 max-w-3xl text-lg leading-8 text-gray-600 dark:text-gray-300">
          Everything on this page happens before the app can help you itself.
          Once Agora is running, its built-in <strong>Help &amp; Guide</strong> takes
          over — and the same guides are{' '}
          <Link href="/docs/guides" className="text-indigo-600 hover:underline dark:text-indigo-400">
            published here
          </Link>{' '}
          as well.
        </p>
        <div className="mt-6 flex flex-wrap gap-3">
          <a
            href={GITHUB_RELEASES_URL}
            className="rounded-lg bg-indigo-600 px-5 py-3 font-semibold text-white hover:bg-indigo-700"
            target="_blank"
            rel="noopener noreferrer"
          >
            Desktop downloads
          </a>
          <Link
            href="/docs/tour"
            className="rounded-lg border px-5 py-3 font-semibold hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800"
          >
            See what it looks like first
          </Link>
        </div>
      </header>

      <section className="rounded-xl border border-sky-300 bg-sky-50 p-5 text-sm leading-6 text-sky-900 dark:border-sky-800 dark:bg-sky-950 dark:text-sky-200">
        <h2 className="font-semibold">Which release do I download?</h2>
        <p className="mt-2">
          Releases tagged <code>v*</code> contain the desktop packages you install.
          Releases tagged <code>registry-*</code> contain catalog data assets, not
          installers — Agora downloads those for you.
        </p>
      </section>

      <section>
        <h2 className="text-2xl font-bold">First run, in order</h2>
        <ol className="mt-5 space-y-5">
          {STEPS.map((step, index) => (
            <li key={step.title} className="flex gap-4">
              <span
                aria-hidden="true"
                className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-indigo-600 text-sm font-bold text-white"
              >
                {index + 1}
              </span>
              <div>
                <h3 className="font-semibold">{step.title}</h3>
                <p className="mt-1 leading-7 text-gray-600 dark:text-gray-300">{step.body}</p>
              </div>
            </li>
          ))}
        </ol>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">Before you start, two things worth knowing</h2>
        <div className="grid gap-4 md:grid-cols-2">
          <article className="rounded-xl border p-5 dark:border-gray-700">
            <h3 className="font-semibold">Your existing Minecraft is untouched</h3>
            <p className="mt-2 text-sm leading-6 text-gray-600 dark:text-gray-300">
              Creating an Agora instance does not replace or modify your normal
              Minecraft installation. Changes stay inside the instance you
              selected.
            </p>
          </article>
          <article className="rounded-xl border p-5 dark:border-gray-700">
            <h3 className="font-semibold">Agora does not have to launch the game</h3>
            <p className="mt-2 text-sm leading-6 text-gray-600 dark:text-gray-300">
              By default Agora prepares the instance and hands it to the official
              Minecraft Launcher, which keeps your Microsoft account and game
              startup where they already are. Launching inside Agora is an
              explicit, optional choice.
            </p>
          </article>
        </div>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">Where to go next</h2>
        <div className="grid gap-4 sm:grid-cols-2">
          <Link
            href="/docs/guides/getting-started"
            className="rounded-xl border bg-white p-5 shadow-sm transition hover:border-indigo-400 dark:border-gray-700 dark:bg-gray-800"
          >
            <h3 className="font-semibold text-indigo-700 dark:text-indigo-400">
              Getting started with Agora
            </h3>
            <p className="mt-1 text-sm leading-6 text-gray-600 dark:text-gray-300">
              The full first-session guide, including a safe path to your first
              playable instance.
            </p>
          </Link>
          <Link
            href="/docs/guides/modding-foundations"
            className="rounded-xl border bg-white p-5 shadow-sm transition hover:border-indigo-400 dark:border-gray-700 dark:bg-gray-800"
          >
            <h3 className="font-semibold text-indigo-700 dark:text-indigo-400">
              Modding foundations
            </h3>
            <p className="mt-1 text-sm leading-6 text-gray-600 dark:text-gray-300">
              Versions, loaders, and dependencies — the three things most
              modding problems come down to.
            </p>
          </Link>
          <Link
            href="/docs/guides/privacy-offline"
            className="rounded-xl border bg-white p-5 shadow-sm transition hover:border-indigo-400 dark:border-gray-700 dark:bg-gray-800"
          >
            <h3 className="font-semibold text-indigo-700 dark:text-indigo-400">
              Privacy, networking, and offline use
            </h3>
            <p className="mt-1 text-sm leading-6 text-gray-600 dark:text-gray-300">
              Every network category Agora can use, how to disable them, and how
              to prepare for playing offline.
            </p>
          </Link>
          <Link
            href="/docs/troubleshooting"
            className="rounded-xl border bg-white p-5 shadow-sm transition hover:border-indigo-400 dark:border-gray-700 dark:bg-gray-800"
          >
            <h3 className="font-semibold text-indigo-700 dark:text-indigo-400">
              Troubleshooting
            </h3>
            <p className="mt-1 text-sm leading-6 text-gray-600 dark:text-gray-300">
              If the first launch does not go well, start with the symptom and
              change one variable at a time.
            </p>
          </Link>
        </div>
      </section>
    </div>
  );
}
