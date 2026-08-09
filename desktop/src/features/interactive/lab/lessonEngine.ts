/**
 * Deterministic Lab lesson engine.
 *
 * Sol-0 contract: `docs/interactive/MASTER_ARCHITECTURE.md` §6.
 *
 *   scenario + current stage + player decision -> next simulated scene + feedback
 *
 * The engine has no clock-dependent success condition and no external
 * service. Animation observes reducer events; animation completion never
 * advances the lesson. Progress is earned by decisions, not by waiting.
 *
 * Lab code must not import Tauri, `lib/tauri`, `live/`, or current operation
 * components — enforced by `scripts/check-interactive-boundaries.mjs`.
 */

import type { LabEvent, LabLessonState, LabReduction, LabScenario } from './scenarioTypes';

export function initialLessonState<Scene>(
  scenario: LabScenario<Scene>,
  checkpoint = 0,
): LabLessonState<Scene> {
  return {
    scenarioId: scenario.id,
    scenarioVersion: scenario.version,
    checkpoint,
    scene: scenario.initialScene(checkpoint),
    status: 'in-progress',
    lastFeedback: null,
  };
}

/**
 * Pure reducer. `reset` restores the canonical first checkpoint; `exit` is
 * handled by the shell (navigation), never by the reducer itself.
 */
export function reduceLesson<Scene>(
  scenario: LabScenario<Scene>,
  state: LabLessonState<Scene>,
  event: LabEvent,
): LabReduction<Scene> {
  switch (event.kind) {
    case 'reset':
      return { state: initialLessonState(scenario), feedback: null };
    case 'exit':
      // The shell owns navigation. Keeping state unchanged here makes the
      // reducer total and the exit behavior shell-observable.
      return { state, feedback: null };
    case 'decision':
      return scenario.reduce(state, event);
  }
}

export function isLessonComplete<Scene>(
  scenario: LabScenario<Scene>,
  state: LabLessonState<Scene>,
): boolean {
  return scenario.successPredicate(state);
}
