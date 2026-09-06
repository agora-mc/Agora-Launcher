import Image from 'next/image';
import Link from 'next/link';

export const metadata = {
  title: 'A visual tour of Agora — Agora Docs',
  description:
    'Annotated screenshots of onboarding, instance creation, install review, loader repair, Crash Doctor, and privacy controls.',
};

const SHOTS = [
  {
    src: '/screenshots/onboarding-welcome.png',
    alt: 'Agora welcome screen with the Get Started button and onboarding summary.',
    title: 'Onboarding',
    body: 'The first thing you see. Onboarding asks which optional services Agora may use, finds Java, and offers to download the signed registry. Every choice here is reversible in Settings.',
    caption: 'Agora 0.1.0, captured 2026-08-05 with a sanitized fixture.',
    href: '/docs/guides/getting-started',
    hrefLabel: 'Getting started guide',
  },
  {
    src: '/screenshots/create-instance.png',
    alt: 'Create Custom Instance dialog showing Minecraft, loader, loader version, and memory controls.',
    title: 'Creating an instance',
    body: 'An instance is one isolated Minecraft setup with its own version, mod loader, mods, and memory. Keeping setups separate is what stops one experiment from breaking another.',
    caption: 'Agora 0.1.0, captured 2026-08-05 with synthetic data.',
    href: '/docs/guides/instances',
    hrefLabel: 'Instances guide',
  },
  {
    src: '/screenshots/install-plan-review.png',
    alt: 'Review Instance Changes dialog showing a summary of what changes, optional extras, dependencies, and a restore-point note.',
    title: 'Reviewing changes before they happen',
    body: 'Agora plans a content change before touching the instance, then shows you what will change, the dependencies it pulled in, the conflicts it found, and the restore point protecting the operation. The exact filenames stay one click away under Technical details.',
    caption: 'Agora 0.1.0, captured 2026-08-05 with synthetic dependencies.',
    href: '/docs/guides/install-update',
    hrefLabel: 'Installing and updating guide',
  },
  {
    src: '/screenshots/loader-compatibility-repair.png',
    alt: 'Health Check dialog showing the current loader, recommended version, and Switch and launch action.',
    title: 'Health checks and loader repair',
    body: 'Before launch, Agora checks the instance. When enabled mods need a loader version you do not have, it names a signed version that satisfies them rather than telling you to guess.',
    caption: 'Agora 0.1.0, captured 2026-08-05 with a synthetic instance.',
    href: '/docs/guides/launching',
    hrefLabel: 'Launching guide',
  },
  {
    src: '/screenshots/crash-doctor.png',
    alt: 'Crash Doctor showing a matched signature, ranked suspects, and bounded crash evidence.',
    title: 'Crash Doctor',
    body: 'When the game crashes, Crash Doctor reads the crash report, matches it against known signatures, and ranks the mods most likely responsible — so you test suspects instead of the whole list.',
    caption: 'Agora 0.1.0, captured 2026-08-05 with a synthetic crash log.',
    href: '/docs/guides/crash-recovery',
    hrefLabel: 'Crash recovery guide',
  },
  {
    src: '/screenshots/privacy-lockdown.png',
    alt: 'Privacy settings showing endpoint controls and the Lockdown Mode toggle.',
    title: 'Privacy and Lockdown Mode',
    body: 'Each kind of network request Agora can make has its own switch, enforced in the backend. Lockdown Mode turns all of them off at once, which is also how you force fully offline behavior.',
    caption: 'Agora 0.1.0, captured 2026-08-05 with no account data.',
    href: '/docs/guides/privacy-offline',
    hrefLabel: 'Privacy and offline guide',
  },
];

export default function TourPage() {
  return (
    <div className="min-w-0 space-y-10">
      <header>
        <nav aria-label="Breadcrumb" className="mb-4 text-sm text-gray-500 dark:text-gray-400">
          <Link href="/docs" className="hover:text-indigo-600 dark:hover:text-indigo-400">
            Docs
          </Link>
          <span aria-hidden="true"> / </span>
          <span className="text-gray-700 dark:text-gray-300">Take a tour</span>
        </nav>
        <h1 className="text-3xl font-bold tracking-tight">A visual tour of Agora</h1>
        <p className="mt-3 max-w-3xl text-lg leading-8 text-gray-600 dark:text-gray-300">
          Six screens that cover most of what Agora does, in the order you would
          meet them. Each one links to the guide that explains it properly.
        </p>
        <p className="mt-3 text-sm text-gray-500 dark:text-gray-400">
          Every capture uses fixed synthetic data. None contain usernames,
          tokens, local paths, server addresses, or private packs.
        </p>
      </header>

      <div className="space-y-10">
        {SHOTS.map((shot, index) => (
          <section key={shot.src}>
            <h2 className="text-2xl font-bold">
              <span className="mr-3 text-gray-400 dark:text-gray-500">{index + 1}</span>
              {shot.title}
            </h2>
            <p className="mt-2 max-w-3xl leading-7 text-gray-600 dark:text-gray-300">{shot.body}</p>
            <figure className="mt-4 overflow-hidden panel-inset">
              <Image
                src={shot.src}
                alt={shot.alt}
                width={1280}
                height={800}
                sizes="(min-width: 1024px) 60vw, 100vw"
                className="h-auto w-full border-b dark:border-gray-700"
              />
              <figcaption className="flex flex-wrap items-center justify-between gap-3 p-3 text-sm text-gray-600 dark:text-gray-300">
                <span>{shot.caption}</span>
                <Link
                  href={shot.href}
                  className="font-medium text-indigo-600 hover:underline dark:text-indigo-400"
                >
                  {shot.hrefLabel} →
                </Link>
              </figcaption>
            </figure>
          </section>
        ))}
      </div>

      <section className="panel-inset p-6">
        <h2 className="text-xl font-bold">Ready to try it?</h2>
        <p className="mt-2 leading-7 text-gray-600 dark:text-gray-300">
          Installing takes a few minutes, and the first-run steps are written
          out in order.
        </p>
        <Link
          href="/docs/install"
          className="btn-gold mt-4 inline-flex px-5 py-3"
        >
          Install Agora
        </Link>
      </section>
    </div>
  );
}
