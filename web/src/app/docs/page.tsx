import Image from 'next/image';
import Link from 'next/link';
import { GITHUB_REPO_URL, GITHUB_RELEASES_URL } from '@/lib/site';

export const metadata = {
  title: 'Documentation - Agora',
  description: 'Install, launch, recover, use offline, and automate Agora safely.',
};

const cards = [
  {
    title: 'Install and first run',
    body: 'Download a packaged release, complete onboarding, synchronize the signed registry, and create or import a disposable first instance.',
  },
  {
    title: 'Install content safely',
    body: 'Browse for the selected instance, choose an exact-compatible artifact, and review dependencies, conflicts, snapshots, and file changes before applying.',
  },
  {
    title: 'Launch and recover',
    body: 'Use delegated launch by default or opt into direct launch. Health checks, loader repair, Crash Doctor, snapshots, and Last Known Good provide different recovery layers.',
  },
  {
    title: 'Privacy and offline play',
    body: 'Agora has no automated analytics, but catalog, downloads, authentication, governance, updates, and optional AI make functional requests when enabled.',
  },
];

const screenshots = [
  {
    src: '/screenshots/onboarding-welcome.png',
    alt: 'Agora welcome screen with the Get Started button and onboarding summary.',
    caption: 'Onboarding welcome in Agora 0.1.0, captured 2026-08-05 with a sanitized fixture.',
  },
  {
    src: '/screenshots/create-instance.png',
    alt: 'Create Custom Instance dialog showing Minecraft, loader, loader version, and memory controls.',
    caption: 'Creating an isolated instance in Agora 0.1.0, captured 2026-08-05 with synthetic data.',
  },
  {
    src: '/screenshots/install-plan-review.png',
    alt: 'Review Instance Changes dialog showing dependencies, a warning, added files, and snapshot details.',
    caption: 'Install-plan review in Agora 0.1.0, captured 2026-08-05 with synthetic dependencies.',
  },
  {
    src: '/screenshots/loader-compatibility-repair.png',
    alt: 'Health Check dialog showing the current loader, recommended version, and Switch and launch action.',
    caption: 'Loader compatibility repair in Agora 0.1.0, captured 2026-08-05 with a synthetic instance.',
  },
  {
    src: '/screenshots/crash-doctor.png',
    alt: 'Crash Doctor showing a matched signature, ranked suspects, and bounded crash evidence.',
    caption: 'Crash Doctor in Agora 0.1.0, captured 2026-08-05 with a synthetic crash log.',
  },
  {
    src: '/screenshots/privacy-lockdown.png',
    alt: 'Privacy settings showing endpoint controls and the documented Lockdown limitation.',
    caption: 'Privacy and Lockdown controls in Agora 0.1.0, captured 2026-08-05 with no account data.',
  },
];

const githubDoc = (path: string) => `${GITHUB_REPO_URL}/blob/HEAD/${path}`;

export default function DocsPage() {
  return (
    <div className="mx-auto max-w-5xl space-y-12">
      <header>
        <p className="text-sm font-semibold uppercase tracking-wide text-indigo-600 dark:text-indigo-400">
          Agora documentation
        </p>
        <h1 className="mt-2 text-4xl font-bold tracking-tight">From download to a recoverable first launch</h1>
        <p className="mt-4 max-w-3xl text-lg text-gray-600 dark:text-gray-300">
          This page covers what you may need before the desktop app is installed. Agora also
          includes a searchable Help &amp; Guide with basic and advanced pages tied to the current
          interface.
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
          <a
            href={githubDoc('docs/TROUBLESHOOTING.md')}
            className="rounded-lg border px-5 py-3 font-semibold hover:bg-gray-100 dark:border-gray-700 dark:hover:bg-gray-800"
            target="_blank"
            rel="noopener noreferrer"
          >
            Troubleshooting reference
          </a>
        </div>
        <p className="mt-3 text-sm text-gray-500 dark:text-gray-400">
          Desktop packages use <code>v*</code> releases. <code>registry-*</code> releases contain data assets, not installers.
        </p>
      </header>

      <section>
        <h2 className="text-2xl font-bold">What Agora helps you do</h2>
        <div className="mt-5 grid gap-4 md:grid-cols-2">
          {cards.map((card) => (
            <article key={card.title} className="rounded-xl border bg-white p-5 shadow-sm dark:border-gray-700 dark:bg-gray-800">
              <h3 className="text-lg font-semibold">{card.title}</h3>
              <p className="mt-2 text-sm leading-6 text-gray-600 dark:text-gray-300">{card.body}</p>
            </article>
          ))}
        </div>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">First run</h2>
        <ol className="list-decimal space-y-3 pl-6 text-gray-700 dark:text-gray-300">
          <li>Download the installer or portable package for your operating system from a published <code>v*</code> release.</li>
          <li>Review the optional service and privacy choices during onboarding.</li>
          <li>Synchronize the signed Agora registry so Browse and governance data are available.</li>
          <li>Let Agora discover or provision a compatible Java runtime.</li>
          <li>Create a small instance or import a supported pack into a disposable test instance.</li>
          <li>Review health findings, then play for at least 60 seconds before treating the setup as known good.</li>
        </ol>
      </section>

      <section className="space-y-5">
        <div>
          <h2 className="text-2xl font-bold">Current interface</h2>
          <p className="mt-2 text-gray-600 dark:text-gray-300">
            These captures use the current Agora 0.1.0 React interface with fixed synthetic data.
            They contain no usernames, tokens, local paths, server addresses, or private packs.
          </p>
        </div>
        <div className="grid gap-6 lg:grid-cols-2">
          {screenshots.map((screenshot) => (
            <figure key={screenshot.src} className="overflow-hidden rounded-xl border bg-white shadow-sm dark:border-gray-700 dark:bg-gray-800">
              <Image
                src={screenshot.src}
                alt={screenshot.alt}
                width={1280}
                height={800}
                sizes="(min-width: 1024px) 50vw, 100vw"
                className="h-auto w-full border-b dark:border-gray-700"
              />
              <figcaption className="p-3 text-sm leading-5 text-gray-600 dark:text-gray-300">
                {screenshot.caption}
              </figcaption>
            </figure>
          ))}
        </div>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">Launch modes</h2>
        <div className="grid gap-4 md:grid-cols-2">
          <article className="rounded-xl border p-5 dark:border-gray-700">
            <h3 className="font-semibold">Delegated launch</h3>
            <p className="mt-2 text-sm leading-6 text-gray-600 dark:text-gray-300">
              The global default. Agora prepares the instance and hands it to the official
              Minecraft Launcher, which owns account authentication and game process startup.
              Instances stored with Auto follow this choice; the current UI does not expose a
              per-instance override selector.
            </p>
          </article>
          <article className="rounded-xl border p-5 dark:border-gray-700">
            <h3 className="font-semibold">Direct launch</h3>
            <p className="mt-2 text-sm leading-6 text-gray-600 dark:text-gray-300">
              Optional. Agora uses the Microsoft account connected inside Agora, selects Java,
              starts Minecraft, and shows process status and live console output.
            </p>
          </article>
        </div>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">Health and loader repair</h2>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          Loader repair shows the current loader, a <strong>Recommended version</strong> when one signed
          candidate satisfies every understood hard requirement, and <strong>Choose compatible version</strong>
          for alternatives. Launch flows use <strong>Switch and launch</strong>; review-only flows use <strong>Switch
          version</strong>. If no signed version satisfies every enabled mod, automatic switching is
          unavailable. Manual candidates are signed but indeterminate and require confirmation in
          the instance editor.
        </p>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          Forge and NeoForge provide language capabilities such as <code>javafml</code> and <code>lowcodefml</code> when
          the active loader exposes the required version. They are not ordinary missing mod JARs.
        </p>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">Recovery is layered</h2>
        <ul className="list-disc space-y-2 pl-6 text-gray-700 dark:text-gray-300">
          <li>Install, update, and remove transactions create full recovery snapshots, including saves.</li>
          <li>Automatic pre-launch recovery protects mod and configuration state, not <code>saves/</code>.</li>
          <li>Last Known Good promotes the exact pre-launch snapshot after a successful session of at least 60 seconds.</li>
          <li>Loadouts remember enabled state; they do not restore versions or configuration.</li>
          <li>Lockfiles describe reproducible artifacts; they are not world backups.</li>
        </ul>
        <p className="rounded-lg border border-amber-300 bg-amber-50 p-4 text-sm text-amber-900 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-200">
          Back up valuable worlds separately. No launcher-local recovery feature should be the only
          copy of irreplaceable save data.
        </p>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">Privacy and offline behavior</h2>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          Offline readiness is instance-specific. Cache the registry, Java runtime, game, loader,
          and mod artifacts, then test the exact launch mode before disconnecting. Individual
          Privacy endpoint switches are enforced by the backend.
        </p>
        <p className="rounded-lg border border-red-300 bg-red-50 p-4 text-sm text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-200">
          Known defect: the current Lockdown toggle persists its UI state and disables the endpoint
          controls, but the backend does not read it as a global network block. Disable every
          endpoint individually before enabling Lockdown, or enforce offline behavior outside Agora.
        </p>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">Integrated AI and MCP</h2>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          The optional integrated assistant uses GitHub Copilot. Assistant messages leave the
          machine. A Crash Doctor handoff can also send the instance ID, selected crash log, matched
          signatures, and ranked suspects in the first message; a mod-list request can send installed
          mod context. Review logs before submitting them.
        </p>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          The optional desktop MCP server binds only to <code>127.0.0.1:39741</code>. It exposes SSE at <code>/sse</code>
          and streamable HTTP at <code>/mcp</code>; every request requires <code>Authorization: Bearer &lt;token&gt;</code>.
          Query-string tokens are not accepted. Its ten tools inspect instances and mods, read bounded
          crash evidence, search local signatures and knowledge, suggest incompatibilities, and request
          approved mod enable/disable actions. The standalone CLI separately offers <code>mcp serve --stdio</code>.
        </p>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">Import and export</h2>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          <strong>Import Pack</strong> accepts <code>.mrpack</code>, <code>.agora-pack.json</code>, and supported JSON pack files. The
          editor exports <strong>Modrinth Pack (.mrpack)</strong> for launcher interoperability and <strong>Agora Pack
          (.json)</strong>, written with an <code>.agora-pack.json</code> filename, for Agora-native metadata. A
          reproduction lockfile is a separate advanced format and never includes world saves.
        </p>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">Data, logs, versions, and support</h2>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          Use <code>agora paths</code> or an instance's <strong>Open in Folder</strong> action instead of hardcoding platform
          paths. Game logs, crash reports, CLI diagnostics, and compiler workflow logs serve different
          purposes and can contain different personal data. Agora does not currently provide a
          one-click support bundle or an in-app exact-version field.
        </p>
        <a
          href={githubDoc('docs/SUPPORT.md')}
          className="inline-flex font-semibold text-indigo-600 hover:underline dark:text-indigo-400"
          target="_blank"
          rel="noopener noreferrer"
        >
          Read the data and support-evidence reference
        </a>
      </section>

      <section className="space-y-4">
        <h2 className="text-2xl font-bold">CLI and automation</h2>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          Agora includes a standalone CLI for instance management, health checks, registry sync,
          direct launch, snapshots, lockfiles, crash investigation, and stdio MCP. Start by checking
          resolved paths and use an isolated data directory for experiments. Authentication uses the
          operating-system credential store and is not isolated by <code>--data-dir</code>.
        </p>
        <a
          href={githubDoc('docs/CLI.md')}
          className="inline-flex font-semibold text-indigo-600 hover:underline dark:text-indigo-400"
          target="_blank"
          rel="noopener noreferrer"
        >
          Read the CLI reference
        </a>
      </section>

      <section className="rounded-xl border bg-white p-6 dark:border-gray-700 dark:bg-gray-800">
        <h2 className="text-xl font-bold">More documentation</h2>
        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <a href={githubDoc('docs/README.md')} target="_blank" rel="noopener noreferrer" className="font-medium text-indigo-600 hover:underline dark:text-indigo-400">
            Documentation index
          </a>
          <a href={githubDoc('docs/TROUBLESHOOTING.md')} target="_blank" rel="noopener noreferrer" className="font-medium text-indigo-600 hover:underline dark:text-indigo-400">
            Troubleshooting
          </a>
          <a href={GITHUB_RELEASES_URL} target="_blank" rel="noopener noreferrer" className="font-medium text-indigo-600 hover:underline dark:text-indigo-400">
            Desktop downloads and release notes
          </a>
          <a href={githubDoc('CODE_OF_ENGAGEMENT.md')} target="_blank" rel="noopener noreferrer" className="font-medium text-indigo-600 hover:underline dark:text-indigo-400">
            Code of Engagement
          </a>
          <Link href="/about" className="font-medium text-indigo-600 hover:underline dark:text-indigo-400">
            How Agora works
          </Link>
        </div>
      </section>
    </div>
  );
}
