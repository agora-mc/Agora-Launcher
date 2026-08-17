import { describe, expect, it } from 'vitest';
import {
  acceptsContinue,
  advanceSatisfiedBy,
  INITIAL_TOUR_STATE,
  TOUR_STEPS,
  tourReducer,
  watchedAnchors,
  type TourStep,
} from './tourModel';

const steps: TourStep[] = [
  { id: 'explain', title: 'Explain', body: '', advance: { kind: 'next' } },
  {
    id: 'go-somewhere',
    title: 'Go',
    body: '',
    anchors: ['nav-browse'],
    advance: { kind: 'appear', anchor: 'page-browse' },
  },
  {
    id: 'click-one',
    title: 'Click',
    body: '',
    anchors: ['nav-guide', 'nav-about'],
    advance: { kind: 'click' },
  },
  {
    id: 'operation',
    title: 'Operation',
    body: '',
    advance: { kind: 'signal', signal: 'install-completed' },
  },
];

const running = (index: number) => ({ status: 'running' as const, index });

describe('tourReducer', () => {
  it('does nothing until the tour is started', () => {
    expect(tourReducer(INITIAL_TOUR_STATE, { type: 'next' }, steps)).toEqual(INITIAL_TOUR_STATE);
    expect(tourReducer(INITIAL_TOUR_STATE, { type: 'skip' }, steps)).toEqual(INITIAL_TOUR_STATE);
    expect(
      tourReducer(INITIAL_TOUR_STATE, { type: 'signal', signal: 'install-completed' }, steps),
    ).toEqual(INITIAL_TOUR_STATE);
  });

  it('starts at the first step, or resumes at a given one', () => {
    expect(tourReducer(INITIAL_TOUR_STATE, { type: 'start' }, steps)).toEqual(running(0));
    expect(tourReducer(INITIAL_TOUR_STATE, { type: 'start', index: 2 }, steps)).toEqual(running(2));
  });

  it('restarts from the beginning when the saved index is out of range', () => {
    expect(tourReducer(INITIAL_TOUR_STATE, { type: 'start', index: 99 }, steps)).toEqual(running(0));
  });

  it('advances an explain-only step on Continue but not on user actions', () => {
    expect(tourReducer(running(0), { type: 'dom-click', anchors: ['nav-browse'] }, steps))
      .toEqual(running(0));
    expect(tourReducer(running(0), { type: 'next' }, steps)).toEqual(running(1));
  });

  it('advances an appear step only for its own anchor', () => {
    expect(tourReducer(running(1), { type: 'next' }, steps)).toEqual(running(1));
    expect(tourReducer(running(1), { type: 'anchor-present', anchor: 'page-home' }, steps))
      .toEqual(running(1));
    expect(tourReducer(running(1), { type: 'anchor-present', anchor: 'page-browse' }, steps))
      .toEqual(running(2));
  });

  it('accepts a click on any of the step anchors', () => {
    expect(tourReducer(running(2), { type: 'dom-click', anchors: ['nav-settings'] }, steps))
      .toEqual(running(2));
    expect(tourReducer(running(2), { type: 'dom-click', anchors: ['nav-about'] }, steps))
      .toEqual(running(3));
  });

  it('advances a signal step only on its own signal', () => {
    expect(tourReducer(running(3), { type: 'signal', signal: 'instance-created' }, steps))
      .toEqual(running(3));
    expect(tourReducer(running(3), { type: 'signal', signal: 'install-completed' }, steps))
      .toEqual({ status: 'finished', index: 3 });
  });

  it('skips forward and steps back without leaving the range', () => {
    expect(tourReducer(running(1), { type: 'skip' }, steps)).toEqual(running(2));
    expect(tourReducer(running(1), { type: 'back' }, steps)).toEqual(running(0));
    expect(tourReducer(running(0), { type: 'back' }, steps)).toEqual(running(0));
  });

  it('finishes after the last step and ends on demand', () => {
    expect(tourReducer(running(3), { type: 'skip' }, steps)).toEqual({ status: 'finished', index: 3 });
    expect(tourReducer(running(2), { type: 'end' }, steps)).toEqual({ status: 'idle', index: 0 });
  });

  it('treats a composite condition as satisfied by any of its parts', () => {
    const composite: TourStep = {
      id: 'either',
      title: '',
      body: '',
      anchors: ['install-instance-select'],
      advance: {
        kind: 'any',
        of: [{ kind: 'change' }, { kind: 'appear', anchor: 'install-version-list' }],
      },
    };
    const only = [composite];
    expect(tourReducer(running(0), { type: 'next' }, only)).toEqual(running(0));
    expect(
      tourReducer(running(0), { type: 'dom-change', anchors: ['install-instance-select'] }, only),
    ).toEqual({ status: 'finished', index: 0 });
    expect(tourReducer(running(0), { type: 'anchor-present', anchor: 'install-version-list' }, only))
      .toEqual({ status: 'finished', index: 0 });
  });
});

describe('advance helpers', () => {
  it('collects every anchor an appear condition watches', () => {
    expect(watchedAnchors({ kind: 'appear', anchor: 'page-browse' })).toEqual(['page-browse']);
    expect(watchedAnchors({ kind: 'next' })).toEqual([]);
    expect(watchedAnchors({
      kind: 'any',
      of: [
        { kind: 'appear', anchor: 'install-open-instance' },
        { kind: 'signal', signal: 'install-completed' },
      ],
    })).toEqual(['install-open-instance']);
  });

  it('offers Continue only where the step accepts it', () => {
    expect(acceptsContinue(steps[0])).toBe(true);
    expect(acceptsContinue(steps[1])).toBe(false);
    expect(acceptsContinue({
      id: 'x',
      title: '',
      body: '',
      advance: { kind: 'any', of: [{ kind: 'next' }, { kind: 'signal', signal: 'install-completed' }] },
    })).toBe(true);
  });

  it('never matches an anchor condition when the step declares no anchors', () => {
    const anchorless: TourStep = { id: 'a', title: '', body: '', advance: { kind: 'click' } };
    expect(advanceSatisfiedBy(anchorless.advance, { type: 'dom-click', anchors: ['nav-home'] }, anchorless))
      .toBe(false);
  });
});

describe('the shipped script', () => {
  it('has unique step ids', () => {
    const ids = TOUR_STEPS.map((step) => step.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('gives every action step something to tell the user to do', () => {
    for (const step of TOUR_STEPS) {
      if (acceptsContinue(step)) continue;
      expect(step.waitingHint, `step "${step.id}" waits on the user with no hint`).toBeTruthy();
    }
  });

  it('gives every gated step an off-track hint', () => {
    for (const step of TOUR_STEPS) {
      if (!step.gate) continue;
      expect(step.offTrackHint, `gated step "${step.id}" has no off-track hint`).toBeTruthy();
    }
  });

  it('reaches the end by performing every step in order', () => {
    // A walk-through of the whole script: each step is satisfied by the event
    // its own condition names, so a step that can never be completed (a typo'd
    // signal, an `appear` on an anchor no step reaches) fails here.
    let state = tourReducer(INITIAL_TOUR_STATE, { type: 'start' });
    for (const step of TOUR_STEPS) {
      expect(state.status).toBe('running');
      expect(TOUR_STEPS[state.index].id).toBe(step.id);
      state = tourReducer(state, eventFor(step));
    }
    expect(state.status).toBe('finished');
  });
});

/** The event a user performing `step` as intended would produce. */
function eventFor(step: TourStep): Parameters<typeof tourReducer>[1] {
  const advance = step.advance.kind === 'any' ? step.advance.of[0] : step.advance;
  switch (advance.kind) {
    case 'next':
      return { type: 'next' };
    case 'appear':
      return { type: 'anchor-present', anchor: advance.anchor };
    case 'signal':
      return { type: 'signal', signal: advance.signal };
    case 'click':
      return { type: 'dom-click', anchors: advance.anchors ?? step.anchors ?? [] };
    case 'change':
      return { type: 'dom-change', anchors: advance.anchors ?? step.anchors ?? [] };
    case 'any':
      throw new Error('nested composite conditions are not part of the model');
  }
}
