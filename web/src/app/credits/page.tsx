const CREDITS = [
  {
    role: 'Creator',
    name: 'Joshua Pfeil',
    url: 'https://joshuapfeil.github.io/',
    body: [
      'I am a programmer, game developer, and gamer. I made Agora because I saw a lot of growing distrust and concerns about the major launchers, while from what I can tell alternatives lack in terms of user friendliness.',
      'I may have built Agora using AI, but personally I made sure every bit is as useful, friendly, and comfortable to use as possible.',
    ],
  },
  {
    role: 'Support and Advice',
    name: "My Mom!",
    body: [
      "Whether it's graphical decisions, user experience, or just simply being there for me, Agora wouldn't be possible without her help. Thanks mom!",
    ],
  },
];

export const metadata = {
  title: "Agora's Credits",
};

export default function CreditsPage() {
  return (
    <div className="mx-auto max-w-3xl space-y-8">
      <div>
        <h1 className="text-3xl font-bold">Agora&apos;s Credits</h1>
        <p className="mt-2 text-gray-600 dark:text-gray-400">
          The people behind Agora.
        </p>
      </div>

      <section className="rounded-xl border border-indigo-200 bg-indigo-50/50 p-6 dark:border-indigo-900 dark:bg-indigo-950/30">
        <p className="text-lg font-medium leading-7">
          Agora is a labor of love, built with the help of AI by a solo developer who cares deeply about the Minecraft
          community. These are the people who made it possible.
        </p>
      </section>

      <section className="space-y-6">
        <h2 className="text-xl font-semibold">Meet the team</h2>
        <ul className="space-y-6">
          {CREDITS.map((credit) => (
            <li
              key={credit.name}
              className="rounded-xl border border-gray-200 p-6 dark:border-gray-700"
            >
              <p className="text-sm font-semibold uppercase tracking-wide text-indigo-700 dark:text-indigo-400">
                {credit.role}
              </p>
              {credit.url ? (
                <h3 className="mt-1 text-2xl font-bold">
                  <a
                    href={credit.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="hover:text-indigo-600 hover:underline dark:text-indigo-400 dark:hover:text-indigo-300"
                  >
                    {credit.name}
                  </a>
                </h3>
              ) : (
                <h3 className="mt-1 text-2xl font-bold">{credit.name}</h3>
              )}
              <div className="mt-3 space-y-3 leading-7 text-gray-700 dark:text-gray-300">
                {credit.body.map((paragraph, index) => (
                  <p key={index}>{paragraph}</p>
                ))}
              </div>
            </li>
          ))}
        </ul>
      </section>

      <section className="space-y-4">
        <h2 className="text-xl font-semibold">Built with the community</h2>
        <p className="leading-7 text-gray-700 dark:text-gray-300">
          Agora is open source and community-curated. Everyone who reviews an entry, casts a vote,
          files a bug, or spreads the word helps keep Agora free, ad-free, and honest. Thank you.
        </p>
      </section>
    </div>
  );
}
