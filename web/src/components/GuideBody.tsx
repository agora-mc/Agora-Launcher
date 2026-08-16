import type { GuideCallout, GuideLevel, GuidePage, GuideSection } from '@/lib/guide';
import { GUIDE_LEVEL_BLURBS, GUIDE_LEVEL_LABELS } from '@/lib/guide';

const CALLOUT_STYLES: Record<GuideCallout['tone'], { box: string; label: string }> = {
  tip: {
    box: 'border-indigo-300 bg-indigo-50 text-indigo-900 dark:border-indigo-800 dark:bg-indigo-950 dark:text-indigo-200',
    label: 'Tip',
  },
  note: {
    box: 'border-sky-300 bg-sky-50 text-sky-900 dark:border-sky-800 dark:bg-sky-950 dark:text-sky-200',
    label: 'Note',
  },
  warning: {
    box: 'border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-200',
    label: 'Careful',
  },
};

function Callout({ callout }: { callout: GuideCallout }) {
  const style = CALLOUT_STYLES[callout.tone];
  return (
    <aside className={`mt-4 rounded-lg border p-4 text-sm leading-6 ${style.box}`}>
      <p className="flex flex-wrap items-baseline gap-2 font-semibold">
        <span className="rounded bg-black/10 px-1.5 py-0.5 text-xs uppercase tracking-wide dark:bg-white/10">
          {style.label}
        </span>
        <span>{callout.title}</span>
      </p>
      <p className="mt-2">{callout.text}</p>
    </aside>
  );
}

function Section({ section }: { section: GuideSection }) {
  return (
    <section className="mt-8">
      <h3 className="text-lg font-semibold">{section.title}</h3>
      <p className="mt-2 leading-7 text-gray-700 dark:text-gray-300">{section.body}</p>
      {section.steps && (
        <ol className="mt-3 list-decimal space-y-2 pl-6 leading-7 text-gray-700 dark:text-gray-300">
          {section.steps.map((step) => (
            <li key={step}>{step}</li>
          ))}
        </ol>
      )}
      {section.bullets && (
        <ul className="mt-3 list-disc space-y-2 pl-6 leading-7 text-gray-700 dark:text-gray-300">
          {section.bullets.map((bullet) => (
            <li key={bullet}>{bullet}</li>
          ))}
        </ul>
      )}
      {section.callout && <Callout callout={section.callout} />}
    </section>
  );
}

/**
 * One level of a guide topic. Both levels are rendered on the page rather than
 * hidden behind tabs: a reader who wants the model instead of the recipe can
 * scroll to it, and neither version is invisible to search or to print.
 */
export function GuideBody({ page, level }: { page: GuidePage; level: GuideLevel }) {
  return (
    <div id={level} className="scroll-mt-8">
      <div className="rounded-xl border bg-white p-5 dark:border-gray-700 dark:bg-gray-800">
        <h2 className="text-2xl font-bold">{GUIDE_LEVEL_LABELS[level]}</h2>
        <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">{GUIDE_LEVEL_BLURBS[level]}</p>
        <p className="mt-4 leading-7 text-gray-700 dark:text-gray-300">{page.summary}</p>
        <div className="mt-4">
          <p className="text-sm font-semibold text-gray-900 dark:text-gray-100">
            After this you will be able to
          </p>
          <ul className="mt-2 list-disc space-y-1 pl-6 text-sm leading-6 text-gray-600 dark:text-gray-300">
            {page.outcomes.map((outcome) => (
              <li key={outcome}>{outcome}</li>
            ))}
          </ul>
        </div>
      </div>

      {page.sections.map((section) => (
        <Section key={section.title} section={section} />
      ))}
    </div>
  );
}
