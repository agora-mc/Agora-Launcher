import { GUIDE_TOPICS } from '@guide/guideContent';
import type {
  GuideCallout,
  GuideLevel,
  GuidePage,
  GuideSection,
  GuideTopic,
} from '@guide/guideContent';

export type { GuideCallout, GuideLevel, GuidePage, GuideSection, GuideTopic };

/**
 * The website's task guides are the desktop app's Help & Guide, rendered for
 * the web. `desktop/src/data/guideContent.ts` is the single source of truth for
 * this copy (see docs/README.md), so the two surfaces cannot contradict each
 * other: the site has no second copy to fall out of date.
 *
 * Every topic ships a `basic` and an `advanced` page. Basic is task-first and
 * assumes no modding background; advanced explains the underlying model for
 * readers who would rather understand the system than follow steps.
 */

/** Order guide categories appear in, roughly the order a new player meets them. */
export const GUIDE_CATEGORY_ORDER: GuideTopic['category'][] = [
  'Start',
  'Play',
  'Manage',
  'Recover',
  'Customize',
  'Connect',
];

export const GUIDE_CATEGORY_BLURBS: Record<GuideTopic['category'], string> = {
  Start: 'Set Agora up and learn the vocabulary before you change anything.',
  Play: 'Build instances, find content, install it safely, and launch.',
  Manage: 'Keep an instance organized, and move setups between machines.',
  Recover: 'Diagnose crashes and roll back to a state that worked.',
  Customize: 'Tune Java, appearance, accessibility, privacy, and offline use.',
  Connect: 'Optional accounts, community governance, and AI integrations.',
};

export const GUIDE_LEVEL_LABELS: Record<GuideLevel, string> = {
  basic: 'Step by step',
  advanced: 'In depth',
};

export const GUIDE_LEVEL_BLURBS: Record<GuideLevel, string> = {
  basic: 'Follow the steps in order. No prior modding knowledge assumed.',
  advanced: 'How the system actually behaves, and why, for readers who prefer the model to a recipe.',
};

export interface GuideCategorySection {
  category: GuideTopic['category'];
  blurb: string;
  topics: GuideTopic[];
}

export function getGuideTopics(): GuideTopic[] {
  return GUIDE_TOPICS;
}

export function getGuideTopic(id: string): GuideTopic | null {
  return GUIDE_TOPICS.find((topic) => topic.id === id) ?? null;
}

export function getGuideTopicIds(): string[] {
  return GUIDE_TOPICS.map((topic) => topic.id);
}

/** Topics bucketed by category, in reading order. Empty categories are dropped. */
export function getGuideCategories(): GuideCategorySection[] {
  return GUIDE_CATEGORY_ORDER.map((category) => ({
    category,
    blurb: GUIDE_CATEGORY_BLURBS[category],
    topics: GUIDE_TOPICS.filter((topic) => topic.category === category),
  })).filter((section) => section.topics.length > 0);
}

/** Previous/next topic in flat reading order, for sequential readers. */
export function getGuideNeighbors(id: string): { prev: GuideTopic | null; next: GuideTopic | null } {
  const ordered = getGuideCategories().flatMap((section) => section.topics);
  const index = ordered.findIndex((topic) => topic.id === id);
  if (index === -1) return { prev: null, next: null };
  return {
    prev: ordered[index - 1] ?? null,
    next: ordered[index + 1] ?? null,
  };
}
