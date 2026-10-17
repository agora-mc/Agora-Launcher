import Link from 'next/link';
import type { ReactNode } from 'react';
import { DownloadButton } from '@/components/DownloadButton';
import { GITHUB_RELEASES_URL } from '@/lib/site';
import { fetchLatestDesktopRelease } from '@/lib/releases';

export const metadata = {
  title: 'Install Agora — Agora Docs',
  description:
    'Download Agora, choose between the desktop app, the portable copy, and the CLI, and read the release file names so you pick the right one.',
};

/** Inline code styling shared by the file-name explanations. */
function Code({ children }: { children: ReactNode }) {
  return (
    <code className="ui-text rounded bg-canvas-deep px-1.5 py-0.5 text-[0.85em] text-gold-bright">
      {children}
    </code>
  );
}

const BULLETS =
  'mt-3 flex-1 space-y-2 pl-6 text-sm leading-6 text-gray-600 dark:text-gray-300 list-disc';

export default async function InstallPage() {
  // Baked in at build time so the button still offers the right installer when
  // the browser-side GitHub API check is rate-limited or blocked.
  const latestRelease = await fetchLatestDesktopRelease();

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
          Agora comes in three shapes — the desktop app, a portable copy, and a command-line
          tool — and all of them live on the same GitHub releases page. This page gets you the
          right download and explains how to read that page when you come back for more.
        </p>
        <div className="mt-6 flex flex-wrap gap-3">
          <Link href="/docs/tour" className="btn-ghost px-5 py-3">
            See what Agora looks like first
          </Link>
          <Link href="/docs/guides" className="btn-ghost px-5 py-3">
            Read the task guides
          </Link>
        </div>
      </header>

      <section aria-labelledby="quick-download" className="panel-inset p-6">
        <h2 id="quick-download" className="rule-gold text-xl font-bold">
          Quick download
        </h2>
        <p className="mt-4 max-w-2xl leading-7 text-gray-600 dark:text-gray-300">
          The button offers the standard desktop installer for the system you are on. If it
          cannot detect your platform, the sections below show how to pick the file by hand.
        </p>
        <div className="mt-5">
          <DownloadButton initialRelease={latestRelease} />
        </div>
      </section>

      <section className="space-y-5">
        <div>
          <h2 className="text-2xl font-bold">Which Agora should I install?</h2>
          <p className="mt-2 max-w-3xl leading-7 text-gray-600 dark:text-gray-300">
            Every release ships all three builds, and they are not mutually exclusive: the app
            and the CLI use the same data root on a machine, so you can add one later without
            moving your instances. The portable copy is the exception — it deliberately keeps
            everything in its own folder.
          </p>
        </div>

        <div className="grid gap-4 lg:grid-cols-3">
          <article className="panel-inset flex flex-col p-5">
            <h3 className="text-lg font-semibold">Desktop app</h3>
            <p className="ui-text mt-1 text-xs font-semibold uppercase tracking-wide text-gold">
              Recommended for most users
            </p>
            <ul className={BULLETS}>
              <li>
                The standard, installed experience: Start menu entries on Windows, Applications
                on macOS, your package manager on Linux.
              </li>
              <li>
                Keeps saved sign-ins in your operating system&rsquo;s credential store, alongside
                your other apps.
              </li>
              <li>
                Has an update check built in, so newer versions are offered from inside the app.
              </li>
              <li>
                Pick this unless you have a specific reason for one of the other two choices.
              </li>
            </ul>
          </article>

          <article className="panel-inset flex flex-col p-5">
            <h3 className="text-lg font-semibold">Portable</h3>
            <p className="ui-text mt-1 text-xs font-semibold uppercase tracking-wide text-gold">
              Windows only · nothing to install
            </p>
            <ul className={BULLETS}>
              <li>
                Everything Agora needs is inside one ZIP. Extract it into any writable folder
                and run it — nothing is installed, and there is nothing to uninstall.
              </li>
              <li>
                Application data lives in a <Code>data</Code> folder beside the executable, so
                the whole setup fits on a thumb drive and works on machines you do not control.
              </li>
              <li>
                Reduced security, by design: saved sign-ins are encrypted inside that folder
                instead of your operating system&rsquo;s credential store, so anyone who can
                read the drive can read them. Keep the drive safe.
              </li>
              <li>
                Updates are manual: close Agora, replace the executable from a newer portable
                ZIP, and keep <Code>portable.txt</Code> and your <Code>data</Code> folder. Do
                not run the installer to update a portable copy.
              </li>
            </ul>
          </article>

          <article className="panel-inset flex flex-col p-5">
            <h3 className="text-lg font-semibold">CLI</h3>
            <p className="ui-text mt-1 text-xs font-semibold uppercase tracking-wide text-gold">
              Advanced users and AI agents
            </p>
            <ul className={BULLETS}>
              <li>
                The <Code>agora</Code> command in a terminal, shipped as a standalone archive
                with no installer. Unpack it and put the binary wherever you keep tools.
              </li>
              <li>
                The same core as the desktop app — instances, installs, health checks, launches,
                snapshots — plus <Code>--json</Code> output for scripts.
              </li>
              <li>
                Made for scripting, support diagnostics, and local AI agents through MCP.
              </li>
              <li>
                Everyone else should stay on the desktop app. The CLI can change the same data
                as the desktop app, so do not run both against one instance at the same time,
                and read the safety rules before experimenting.
              </li>
            </ul>
            <p className="mt-4 text-sm font-semibold text-indigo-600 dark:text-indigo-400">
              <Link href="/docs/cli" className="hover:underline">
                CLI reference
              </Link>
              <span aria-hidden="true"> · </span>
              <Link href="/docs/guides/mcp-automation" className="hover:underline">
                MCP and external AI tools
              </Link>
            </p>
          </article>
        </div>
      </section>

      <section className="space-y-5">
        <div>
          <h2 className="text-2xl font-bold">Understanding the releases page</h2>
          <p className="mt-2 max-w-3xl leading-7 text-gray-600 dark:text-gray-300">
            The releases page holds two different things, and only one of them is something you
            install. Knowing the two apart is the whole trick.
          </p>
        </div>

        <div className="panel-inset p-5">
          <h3 className="font-semibold">App releases and catalog releases</h3>
          <ul className="mt-3 list-disc space-y-3 pl-6 text-sm leading-6 text-gray-600 dark:text-gray-300">
            <li>
              <Code>v1.2.3</Code> — an <strong className="text-gray-700 dark:text-gray-300">app
              release</strong>. The tag starting with <Code>v</Code> is the version, and the
              release carries the installers and archives for it. This is the family to install
              from.
            </li>
            <li>
              <Code>registry-2026-08-27</Code> — a{' '}
              <strong className="text-gray-700 dark:text-gray-300">catalog snapshot</strong>: the
              curated catalog database, published with the date as its tag. Agora downloads and
              verifies those for itself; there is nothing here for you to install, and these
              releases can be ignored entirely.
            </li>
          </ul>
          <p className="mt-4 text-sm leading-6 text-gray-600 dark:text-gray-300">
            GitHub gives its &ldquo;Latest&rdquo; badge to the newest ordinary release, and
            catalog snapshots are ordinary releases — so the badge, and any
            &ldquo;releases/latest&rdquo; link, can point at a catalog snapshot. Ignore the
            badge: open the newest release whose tag starts with <Code>v</Code>.
          </p>
        </div>

        <div className="panel-inset p-5">
          <h3 className="font-semibold">What the file names mean</h3>
          <p className="mt-2 text-sm leading-6 text-gray-600 dark:text-gray-300">
            Every app release follows one pattern, so the important parts are readable before
            you download anything:
          </p>
          <p className="mt-3">
            <Code>agora-desktop-v1.2.3-windows-x64-setup.exe</Code>
          </p>
          <dl className="mt-4 grid gap-x-6 gap-y-3 text-sm leading-6 sm:grid-cols-2">
            <div>
              <dt className="ui-text font-semibold text-gold-bright">agora</dt>
              <dd className="text-gray-600 dark:text-gray-300">
                The product name. Every download starts with it.
              </dd>
            </div>
            <div>
              <dt className="ui-text font-semibold text-gold-bright">desktop / cli</dt>
              <dd className="text-gray-600 dark:text-gray-300">
                Which Agora this is: the desktop app, or the command-line tool.
              </dd>
            </div>
            <div>
              <dt className="ui-text font-semibold text-gold-bright">v1.2.3</dt>
              <dd className="text-gray-600 dark:text-gray-300">
                The version. Every file in one release carries the same one, so you never have
                to mix versions.
              </dd>
            </div>
            <div>
              <dt className="ui-text font-semibold text-gold-bright">windows / macos / linux</dt>
              <dd className="text-gray-600 dark:text-gray-300">
                The operating system the file is built for.
              </dd>
            </div>
            <div>
              <dt className="ui-text font-semibold text-gold-bright">x64 / arm64</dt>
              <dd className="text-gray-600 dark:text-gray-300">
                The processor architecture. <Code>x64</Code> covers Intel and AMD machines
                (including Intel Macs); <Code>arm64</Code> is Apple Silicon. Older releases
                write these as <Code>x86_64</Code> and <Code>aarch64</Code>.
              </dd>
            </div>
            <div>
              <dt className="ui-text font-semibold text-gold-bright">
                setup / msi / portable / dmg / AppImage / deb / rpm
              </dt>
              <dd className="text-gray-600 dark:text-gray-300">
                The package format. The next section says which to pick.
              </dd>
            </div>
          </dl>
        </div>

        <div>
          <h3 className="text-lg font-semibold">Which file do I pick for my system?</h3>
          <div className="mt-4 grid gap-4 sm:grid-cols-2">
            <article className="panel-inset p-5">
              <h4 className="font-semibold">Windows</h4>
              <ul className="mt-2 list-disc space-y-2 pl-6 text-sm leading-6 text-gray-600 dark:text-gray-300">
                <li>
                  <Code>...-windows-x64-setup.exe</Code> — the standard installer. Start here.
                </li>
                <li>
                  <Code>...-windows-x64.msi</Code> — the same app packaged as an MSI, for
                  managed or scripted installs.
                </li>
                <li>
                  <Code>...-windows-x64-portable.zip</Code> — no install; see Portable above.
                </li>
              </ul>
            </article>
            <article className="panel-inset p-5">
              <h4 className="font-semibold">macOS</h4>
              <ul className="mt-2 list-disc space-y-2 pl-6 text-sm leading-6 text-gray-600 dark:text-gray-300">
                <li>
                  <Code>...-macos-arm64.dmg</Code> — Apple Silicon (M-series chips).
                </li>
                <li>
                  <Code>...-macos-x64.dmg</Code> — Intel Macs.
                </li>
                <li>
                  Not sure which you have? Apple menu → About This Mac: &ldquo;Chip&rdquo; means
                  Apple Silicon, &ldquo;Processor&rdquo; means Intel.
                </li>
              </ul>
            </article>
            <article className="panel-inset p-5">
              <h4 className="font-semibold">Linux</h4>
              <ul className="mt-2 list-disc space-y-2 pl-6 text-sm leading-6 text-gray-600 dark:text-gray-300">
                <li>
                  <Code>...-linux-x64.AppImage</Code> — runs on most distributions; mark it
                  executable and run it.
                </li>
                <li>
                  <Code>...-linux-x64.deb</Code> — Debian, Ubuntu, Mint, Pop!_OS, and friends.
                </li>
                <li>
                  <Code>...-linux-x64.rpm</Code> — Fedora, RHEL, openSUSE, and friends.
                </li>
              </ul>
            </article>
            <article className="panel-inset p-5">
              <h4 className="font-semibold">Command line</h4>
              <ul className="mt-2 list-disc space-y-2 pl-6 text-sm leading-6 text-gray-600 dark:text-gray-300">
                <li>
                  <Code>agora-cli-v1.2.3-windows-x64.zip</Code> on Windows, and{' '}
                  <Code>.tar.gz</Code> archives for <Code>linux-x64</Code>,{' '}
                  <Code>macos-x64</Code>, and <Code>macos-arm64</Code>.
                </li>
                <li>
                  Unpack it and put the <Code>agora</Code> binary wherever you keep tools —
                  there is no installer.
                </li>
              </ul>
            </article>
          </div>
        </div>

        <div className="panel-inset p-5">
          <h3 className="font-semibold">The files that are not installs</h3>
          <ul className="mt-3 list-disc space-y-2 pl-6 text-sm leading-6 text-gray-600 dark:text-gray-300">
            <li>
              <Code>...-macos-arm64.app.tar.gz</Code> (and its <Code>x64</Code> twin) — macOS
              updater archives. Agora&rsquo;s update check fetches these itself; you never
              install from them by hand.
            </li>
            <li>
              <Code>latest.json</Code> — the metadata that update check reads. You never need
              to download it yourself.
            </li>
            <li>
              <Code>SHA256SUMS</Code> — one list of SHA-256 hashes covering the installers and
              archives in the release. Optional but quick: on Linux or macOS, with the release
              files in one folder, run <Code>sha256sum -c SHA256SUMS</Code>; on Windows, compare
              a file&rsquo;s <Code>Get-FileHash</Code> output against the list.
            </li>
          </ul>
        </div>

        <p className="text-sm leading-6 text-gray-600 dark:text-gray-300">
          Everything above lives on{' '}
          <a
            href={GITHUB_RELEASES_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="font-semibold text-indigo-600 hover:underline dark:text-indigo-400"
          >
            the releases page
          </a>
          . Newer app releases open with a direct-download table in their notes. Underneath,
          GitHub keeps the full asset list alphabetical (CLI builds before desktop builds) and
          folds it behind &ldquo;Show all assets&rdquo; when it is long — expand it for files
          the table does not cover, such as the <Code>.msi</Code> or <Code>.deb</Code>
          installers.
        </p>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">Where to go next</h2>
        <div className="grid gap-4 sm:grid-cols-2">
          <Link href="/docs/guides/getting-started" className="panel-inset card-lift p-5">
            <h3 className="font-semibold text-indigo-700 dark:text-indigo-400">
              Getting started with Agora
            </h3>
            <p className="mt-1 text-sm leading-6 text-gray-600 dark:text-gray-300">
              The full first session, ending in a playable instance and a recovery point.
            </p>
          </Link>
          <Link href="/docs/cli" className="panel-inset card-lift p-5">
            <h3 className="font-semibold text-indigo-700 dark:text-indigo-400">
              CLI reference
            </h3>
            <p className="mt-1 text-sm leading-6 text-gray-600 dark:text-gray-300">
              Every command, flag, and exit code — plus the safety rules before you experiment.
            </p>
          </Link>
          <Link
            href="/docs/guides/mcp-automation"
            className="panel-inset card-lift p-5"
          >
            <h3 className="font-semibold text-indigo-700 dark:text-indigo-400">
              MCP and external AI tools
            </h3>
            <p className="mt-1 text-sm leading-6 text-gray-600 dark:text-gray-300">
              Connect a local AI client, and decide exactly what it may touch.
            </p>
          </Link>
          <Link href="/docs/troubleshooting" className="panel-inset card-lift p-5">
            <h3 className="font-semibold text-indigo-700 dark:text-indigo-400">
              Troubleshooting
            </h3>
            <p className="mt-1 text-sm leading-6 text-gray-600 dark:text-gray-300">
              If install or first launch does not go well, start from the symptom here.
            </p>
          </Link>
        </div>
      </section>
    </div>
  );
}
