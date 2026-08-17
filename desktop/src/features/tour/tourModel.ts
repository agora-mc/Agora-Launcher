/**
 * Guided walkthrough — step model and the walkthrough script.
 *
 * The tour teaches the launcher by having the user do the real thing: each
 * step highlights a piece of the live UI and then waits for the user's own
 * action — a click, a form change, or a completed operation — before moving
 * on. Nothing here simulates the app (Agora Lab owns the pretend-copy
 * teaching surface); this walks the real one.
 *
 * Anchors are a DOM opt-in: an element carries `data-tour="<anchor>"` and the
 * overlay finds it with a single `querySelector`. Steps therefore never import
 * page components and never read app state, so instrumenting a new surface
 * costs exactly one attribute. `tourAnchors.test.ts` fails the build when a
 * step names an anchor no element declares.
 *
 * Everything in this file is pure and framework-free so the step machine can
 * be tested without a DOM; `TourProvider` translates browser events into the
 * `TourEvent` union below.
 */

/** The value of a `data-tour` attribute. Kebab-case, matched exactly. */
export type TourAnchor = string;

/**
 * Operations the app reports to the tour because their completion is not
 * observable from the DOM alone (a dialog closing does not tell us whether the
 * work succeeded or the user cancelled).
 */
export type TourSignal = 'instance-created' | 'install-completed';

export type TourAdvance =
  /** The card's Continue button — used for steps that only explain. */
  | { kind: 'next' }
  /** A real click inside one of the step's anchors (or `anchors`, if given). */
  | { kind: 'click'; anchors?: readonly TourAnchor[] }
  /** A form control inside one of the step's anchors changed. */
  | { kind: 'change'; anchors?: readonly TourAnchor[] }
  /** An anchor is present in the DOM (a page, dialog, or panel opened). */
  | { kind: 'appear'; anchor: TourAnchor }
  /** The app reported a completed operation. */
  | { kind: 'signal'; signal: TourSignal }
  /** Any of several conditions — the first one that happens wins. */
  | { kind: 'any'; of: readonly TourAdvance[] };

export interface TourStep {
  id: string;
  title: string;
  body: string;
  /**
   * Elements to spotlight. By default the first anchor that is on screen is
   * highlighted, so a step can name a precise target and fall back to a
   * coarser one (`['install-review-confirm', 'install-review-dialog']`).
   */
  anchors?: readonly TourAnchor[];
  /** Spotlight every anchor that is present instead of only the first. */
  highlightAll?: boolean;
  /**
   * The surface this step belongs to. When it is missing the user has
   * wandered off, so the card shows `offTrackHint` and refuses to continue
   * rather than explaining something that is not on screen.
   */
  gate?: TourAnchor;
  advance: TourAdvance;
  /** Shown while the tour waits for the user's action. */
  waitingHint?: string;
  /** Shown instead when `gate` is off screen. */
  offTrackHint?: string;
  /** Overrides the Continue button label on `next` steps. */
  continueLabel?: string;
}

// ---------------------------------------------------------------------------
// The script
// ---------------------------------------------------------------------------

export const TOUR_STEPS: readonly TourStep[] = [
  {
    id: 'home-page',
    title: 'This is the home page',
    body:
      'Home is your dashboard: the instance you played last, recovery points you can roll back to, '
      + 'registry status, and a few recommendations.',
    anchors: ['page-home'],
    gate: 'page-home',
    advance: { kind: 'next' },
    offTrackHint: 'Open Home in the sidebar to start the tour.',
  },
  {
    id: 'home-nav',
    title: 'Home is always one click away',
    body: 'Wherever you end up, clicking Home in the sidebar brings you back here.',
    anchors: ['nav-home'],
    gate: 'page-home',
    advance: { kind: 'next' },
    offTrackHint: 'Open Home in the sidebar to continue.',
  },
  {
    id: 'instances-nav',
    title: 'Next: make an instance',
    body:
      'An instance is where your mods are organised — you might call it a custom modpack. '
      + 'Open My Instances to make one.',
    anchors: ['nav-instances'],
    advance: { kind: 'appear', anchor: 'page-instances' },
    waitingHint: 'Click “My Instances” in the sidebar.',
  },
  {
    id: 'create-instance-button',
    title: 'Create your first instance',
    body:
      'In Agora you can keep as many instances as you like, each with its own saves, configuration, '
      + 'and mod list. Let’s make one now.',
    anchors: ['create-instance'],
    gate: 'page-instances',
    advance: { kind: 'appear', anchor: 'create-instance-dialog' },
    waitingHint: 'Click “+ Create Instance”.',
    offTrackHint: 'Open My Instances again to continue.',
  },
  {
    id: 'create-instance-form',
    title: 'Name it and leave the rest',
    body:
      'Here you set the name, the Minecraft version, the loader, the loader version, and how much '
      + 'memory Minecraft gets. The defaults are fine for now — just give it a name.',
    anchors: ['create-instance-form'],
    gate: 'create-instance-dialog',
    advance: { kind: 'next' },
    offTrackHint: 'Open “+ Create Instance” again to continue.',
  },
  {
    id: 'create-instance-submit',
    title: 'Create it',
    body: 'When you’re happy with the name, click Create.',
    anchors: ['create-instance-submit'],
    gate: 'create-instance-dialog',
    advance: { kind: 'signal', signal: 'instance-created' },
    waitingHint: 'Waiting for the instance to be created…',
    offTrackHint: 'Open “+ Create Instance” again to continue.',
  },
  {
    id: 'browse-nav',
    title: 'Now it needs mods',
    body: 'Everything you can install lives on the Browse page. Let’s go there.',
    anchors: ['nav-browse'],
    advance: { kind: 'appear', anchor: 'page-browse' },
    waitingHint: 'Click “Browse” in the sidebar.',
  },
  {
    id: 'browse-results',
    title: 'Browse the catalog',
    body:
      'Browse covers every mod, shader, modpack, data pack, resource pack, server, and world in the '
      + 'Agora registry — plus Modrinth, if you enabled it. Browse has two modes to explore; stick '
      + 'with the one you have open for now. Open any item to see its details.',
    anchors: ['browse-results'],
    gate: 'page-browse',
    advance: { kind: 'appear', anchor: 'page-mod-detail' },
    waitingHint: 'Open any item’s details to continue.',
    offTrackHint: 'Go back to Browse to continue.',
  },
  {
    id: 'mod-detail-tabs',
    title: 'What a details page tells you',
    body:
      'About describes the mod, Gallery shows it in action, and Versions lists every release you can '
      + 'install. Curated entries also get an Agora tab with their review history and curated status.',
    anchors: ['mod-detail-tabs'],
    gate: 'page-mod-detail',
    advance: { kind: 'next' },
    offTrackHint: 'Open an item from Browse to continue.',
  },
  {
    id: 'mod-detail-install',
    title: 'Install it into your instance',
    body:
      'Let’s put this mod in the instance you just made. (If you opened a modpack rather than a mod, '
      + 'go back and pick a mod — packs build an instance of their own instead.)',
    anchors: ['mod-detail-install'],
    gate: 'page-mod-detail',
    advance: { kind: 'appear', anchor: 'install-panel' },
    waitingHint: 'Click “Install to Instance”.',
    offTrackHint: 'Open an item from Browse to continue.',
  },
  {
    id: 'install-pick-instance',
    title: 'Choose where it goes',
    body: 'Pick the instance you want this mod installed into.',
    anchors: ['install-instance-select'],
    gate: 'install-panel',
    advance: {
      kind: 'any',
      of: [{ kind: 'change' }, { kind: 'appear', anchor: 'install-version-list' }],
    },
    waitingHint: 'Select your instance from the list.',
    offTrackHint: 'Reopen “Install to Instance” to continue.',
  },
  {
    id: 'install-next-version',
    title: 'Then pick a version',
    body: 'Agora only offers versions that fit the instance’s Minecraft version and loader.',
    anchors: ['install-next-version'],
    gate: 'install-panel',
    advance: { kind: 'appear', anchor: 'install-version-list' },
    waitingHint: 'Click “Next: Choose Version”.',
    offTrackHint: 'Reopen “Install to Instance” to continue.',
  },
  {
    id: 'install-choose-version',
    title: 'Take the newest compatible one',
    body: 'Newest first. Compatible releases are marked, so the top of the list is usually the one you want.',
    anchors: ['install-version-first', 'install-version-list'],
    gate: 'install-panel',
    advance: { kind: 'appear', anchor: 'install-confirm' },
    waitingHint: 'Select a version to continue.',
    offTrackHint: 'Reopen “Install to Instance” to continue.',
  },
  {
    id: 'install-confirm',
    title: 'Start the install',
    body: 'This does not install anything yet — it opens the review first.',
    anchors: ['install-confirm'],
    gate: 'install-panel',
    advance: { kind: 'appear', anchor: 'install-review-dialog' },
    waitingHint: 'Click the install button.',
    offTrackHint: 'Reopen “Install to Instance” to continue.',
  },
  {
    id: 'install-review',
    title: 'Agora reviews the change first',
    body:
      'Before touching your instance, Agora works out what this actually changes — dependencies, '
      + 'conflicts, files replaced — and takes a snapshot it can roll back to. When it’s satisfied, '
      + 'it lets you install.',
    anchors: ['install-review-confirm', 'install-review-dialog'],
    gate: 'install-review-dialog',
    advance: {
      kind: 'any',
      of: [
        { kind: 'appear', anchor: 'install-open-instance' },
        { kind: 'signal', signal: 'install-completed' },
      ],
    },
    waitingHint: 'Confirm the install to continue.',
    offTrackHint: 'Run the install again to continue, or skip this step.',
  },
  {
    id: 'open-instance',
    title: 'Open the instance',
    body: 'The mod is in. Let’s look at the instance you just changed.',
    anchors: ['install-open-instance', 'nav-instances'],
    advance: { kind: 'appear', anchor: 'page-instance-editor' },
    waitingHint: 'Open the instance to continue.',
  },
  {
    id: 'instance-editor',
    title: 'The instance editor',
    body:
      'This lists everything installed, and Play launches it. Like Browse, it has two modes — the one '
      + 'you have open is fine. Everything else here (snapshots, loadouts, import and export, the '
      + 'console) is for when you need it.',
    anchors: ['page-instance-editor'],
    gate: 'page-instance-editor',
    advance: { kind: 'next' },
    offTrackHint: 'Open an instance from My Instances to continue.',
  },
  {
    id: 'reading-pages',
    title: 'Three pages worth reading',
    body:
      'Community Governance, Help & Guide, and The Agora Difference explain how the registry is '
      + 'curated, how to do just about anything in the launcher, and why Agora works the way it does. '
      + 'Open whichever one you like.',
    anchors: ['nav-governance', 'nav-guide', 'nav-about'],
    highlightAll: true,
    advance: { kind: 'click' },
    waitingHint: 'Open any one of the three to continue.',
  },
  {
    id: 'field-guide',
    title: 'Field Guide',
    body:
      'A just-for-fun achievements list. A few of them only turn up if you have the living background '
      + 'switched on.',
    anchors: ['nav-field-guide'],
    advance: { kind: 'next' },
  },
  {
    id: 'settings',
    title: 'Settings',
    body:
      'Settings is where you shape how Agora looks and behaves. There’s a lot in there, so take your '
      + 'time — Appearance and Launch Mode are the two most worth knowing early.',
    anchors: ['nav-settings'],
    advance: { kind: 'next' },
  },
  {
    id: 'finish',
    title: 'That’s the tour',
    body:
      'That’s everything — thanks for following along. You can run this again any time from Settings '
      + 'or from Help & Guide.',
    advance: { kind: 'next' },
    continueLabel: 'Finish',
  },
] as const;

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

export type TourStatus = 'idle' | 'running' | 'finished';

export interface TourState {
  status: TourStatus;
  /** Index into the step list. Meaningful only while `status === 'running'`. */
  index: number;
}

export const INITIAL_TOUR_STATE: TourState = { status: 'idle', index: 0 };

export type TourEvent =
  | { type: 'start'; index?: number }
  | { type: 'next' }
  | { type: 'back' }
  | { type: 'skip' }
  | { type: 'end' }
  /** A click landed inside these anchors (innermost first). */
  | { type: 'dom-click'; anchors: readonly TourAnchor[] }
  /** A `change` event fired inside these anchors (innermost first). */
  | { type: 'dom-change'; anchors: readonly TourAnchor[] }
  /** This anchor is currently in the DOM. */
  | { type: 'anchor-present'; anchor: TourAnchor }
  | { type: 'signal'; signal: TourSignal };

function matchesAnchors(
  wanted: readonly TourAnchor[] | undefined,
  hit: readonly TourAnchor[],
): boolean {
  if (!wanted || wanted.length === 0) return false;
  return wanted.some((anchor) => hit.includes(anchor));
}

/** Whether `event` satisfies `advance` for `step`. */
export function advanceSatisfiedBy(
  advance: TourAdvance,
  event: TourEvent,
  step: TourStep,
): boolean {
  switch (advance.kind) {
    case 'any':
      return advance.of.some((inner) => advanceSatisfiedBy(inner, event, step));
    case 'next':
      return event.type === 'next';
    case 'click':
      return event.type === 'dom-click'
        && matchesAnchors(advance.anchors ?? step.anchors, event.anchors);
    case 'change':
      return event.type === 'dom-change'
        && matchesAnchors(advance.anchors ?? step.anchors, event.anchors);
    case 'appear':
      return event.type === 'anchor-present' && event.anchor === advance.anchor;
    case 'signal':
      return event.type === 'signal' && event.signal === advance.signal;
  }
}

/** Every anchor an `appear` condition on this step watches for. */
export function watchedAnchors(advance: TourAdvance): TourAnchor[] {
  if (advance.kind === 'appear') return [advance.anchor];
  if (advance.kind === 'any') return advance.of.flatMap(watchedAnchors);
  return [];
}

/** True when the step waits on the user rather than on the Continue button. */
export function acceptsContinue(step: TourStep): boolean {
  const accepts = (advance: TourAdvance): boolean =>
    advance.kind === 'next'
    || (advance.kind === 'any' && advance.of.some(accepts));
  return accepts(step.advance);
}

function step(steps: readonly TourStep[], index: number): TourStep | null {
  return steps[index] ?? null;
}

/** Move to `index`, finishing the tour when it runs past the last step. */
function goTo(steps: readonly TourStep[], index: number): TourState {
  if (index >= steps.length) return { status: 'finished', index: steps.length - 1 };
  return { status: 'running', index: Math.max(0, index) };
}

/**
 * The whole state machine. Pure: `TourProvider` feeds it browser events and
 * persists whatever comes back.
 */
export function tourReducer(
  state: TourState,
  event: TourEvent,
  steps: readonly TourStep[] = TOUR_STEPS,
): TourState {
  if (event.type === 'start') {
    const index = event.index ?? 0;
    return goTo(steps, index >= steps.length ? 0 : index);
  }
  if (event.type === 'end') return { status: 'idle', index: 0 };
  if (state.status !== 'running') return state;

  const current = step(steps, state.index);
  if (!current) return { status: 'finished', index: Math.max(0, steps.length - 1) };

  switch (event.type) {
    case 'back':
      return goTo(steps, state.index - 1);
    case 'skip':
      return goTo(steps, state.index + 1);
    default:
      return advanceSatisfiedBy(current.advance, event, current)
        ? goTo(steps, state.index + 1)
        : state;
  }
}
