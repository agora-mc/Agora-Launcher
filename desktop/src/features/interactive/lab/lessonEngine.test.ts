import { describe, expect, it } from 'vitest';
import { initialLessonState, isLessonComplete, reduceLesson } from './lessonEngine';
import type { FeedbackEvent, LabLessonState, LabScenario } from './scenarioTypes';
import { buildItScenario } from './scenarios/buildIt';
import { modItScenario } from './scenarios/modIt';
import { undoItScenario } from './scenarios/undoIt';

function play<Scene>(
  scenario: LabScenario<Scene>,
  decisionIds: string[],
): { state: LabLessonState<Scene>; feedback: FeedbackEvent[] } {
  let state = initialLessonState(scenario);
  const feedback: FeedbackEvent[] = [];
  for (const decisionId of decisionIds) {
    const reduction = reduceLesson(scenario, state, { kind: 'decision', decisionId });
    state = reduction.state;
    if (reduction.feedback) feedback.push(reduction.feedback);
  }
  return { state, feedback };
}

function lastFeedback(feedback: FeedbackEvent[]): FeedbackEvent | undefined {
  return feedback[feedback.length - 1];
}

describe('lessonEngine', () => {
  it('initialLessonState starts at checkpoint 0, in progress', () => {
    const state = initialLessonState(buildItScenario);
    expect(state.checkpoint).toBe(0);
    expect(state.status).toBe('in-progress');
    expect(state.scenarioId).toBe('build');
    expect(state.lastFeedback).toBeNull();
  });

  it('reset restores the initial canonical scene', () => {
    let state = initialLessonState(buildItScenario);
    state = reduceLesson(buildItScenario, state, { kind: 'decision', decisionId: 'place-version' }).state;
    expect(state.checkpoint).toBe(1);
    const reset = reduceLesson(buildItScenario, state, { kind: 'reset' }).state;
    expect(reset.checkpoint).toBe(0);
    expect(reset.status).toBe('in-progress');
  });

  it('exit leaves state unchanged (shell owns navigation)', () => {
    const state = initialLessonState(buildItScenario);
    const exited = reduceLesson(buildItScenario, state, { kind: 'exit' });
    expect(exited.state).toBe(state);
  });
});

describe('Build It scenario', () => {
  it('completes via the Fabric path', () => {
    const { state, feedback } = play(buildItScenario, [
      'place-version',
      'choose-fabric',
      'name-it',
      'place-mod',
    ]);
    expect(isLessonComplete(buildItScenario, state)).toBe(true);
    expect(state.status).toBe('complete');
    expect(feedback.some((event) => event.tone === 'success')).toBe(true);
  });

  it('recovers from the Forge mistake and completes', () => {
    const first = play(buildItScenario, ['place-version', 'choose-forge']);
    expect(first.state.checkpoint).toBe(1);
    expect(lastFeedback(first.feedback)?.tone).toBe('caution');
    expect(isLessonComplete(buildItScenario, first.state)).toBe(false);

    const { state } = play(buildItScenario, [
      'place-version',
      'choose-forge',
      'choose-fabric',
      'name-it',
      'place-mod',
    ]);
    expect(isLessonComplete(buildItScenario, state)).toBe(true);
  });

  it('blocks an incompatible tile and recovers by switching loader', () => {
    const blocked = play(buildItScenario, ['place-version', 'choose-vanilla', 'name-it', 'place-mod']);
    expect(lastFeedback(blocked.feedback)?.tone).toBe('blocked');
    expect(isLessonComplete(buildItScenario, blocked.state)).toBe(false);

    const { state } = play(buildItScenario, [
      'place-version',
      'choose-vanilla',
      'name-it',
      'place-mod',
      'switch-fabric',
      'place-mod',
    ]);
    expect(isLessonComplete(buildItScenario, state)).toBe(true);
  });
});

describe('Mod It scenario', () => {
  it('completes: stage, satisfy requirement, include optional, resolve conflict, apply', () => {
    const { state } = play(modItScenario, [
      'stage-better-caves',
      'add-core-lib',
      'include-nice-textures',
      'replace-terrain-overhaul',
      'apply-plan',
    ]);
    expect(isLessonComplete(modItScenario, state)).toBe(true);
    // committed proposal is removed after apply (fresh read outcome)
    expect(state.scene.proposals).toHaveLength(0);
  });

  it('a blocking conflict prevents apply until resolved', () => {
    const { state } = play(modItScenario, [
      'stage-better-caves',
      'add-core-lib',
      'include-nice-textures',
      'apply-plan',
    ]);
    // apply-plan is ignored while a blocking conflict remains
    expect(state.status).toBe('in-progress');
    expect(isLessonComplete(modItScenario, state)).toBe(false);
  });

  it('keep-current is a recoverable mistake', () => {
    const kept = play(modItScenario, ['stage-better-caves', 'add-core-lib', 'include-nice-textures', 'keep-current']);
    expect(lastFeedback(kept.feedback)?.tone).toBe('caution');
    expect(isLessonComplete(modItScenario, kept.state)).toBe(false);

    const { state } = play(modItScenario, [
      'stage-better-caves',
      'add-core-lib',
      'include-nice-textures',
      'keep-current',
      'replace-terrain-overhaul',
      'apply-plan',
    ]);
    expect(isLessonComplete(modItScenario, state)).toBe(true);
  });

  it('maps visual intents to scenario decisions', () => {
    const scene = modItScenario.initialScene(0);
    const staged = modItScenario.intentToDecision?.(scene, {
      kind: 'propose-install',
      contentId: 'lab:mod:better-caves',
    });
    expect(staged).toEqual({ kind: 'decision', decisionId: 'stage-better-caves' });
    const ignored = modItScenario.intentToDecision?.(scene, { kind: 'select', entityId: 'lab:mod:core-lib' });
    expect(ignored).toBeNull();
  });

  it('after apply, content is fully current with no proposal markers', () => {
    const { state } = play(modItScenario, [
      'stage-better-caves',
      'add-core-lib',
      'include-nice-textures',
      'replace-terrain-overhaul',
      'apply-plan',
    ]);
    const byId = (id: string) => state.scene.content.find((node) => node.id === id);
    expect(byId('lab:mod:better-caves')?.presence).toEqual({ current: 'installed' });
    expect(byId('lab:mod:core-lib')?.presence).toEqual({ current: 'installed' });
    expect(byId('lab:mod:nice-textures')?.presence).toEqual({ current: 'installed' });
    expect(byId('lab:mod:terrain-overhaul')?.presence).toEqual({ current: 'not-installed' });
    expect(state.scene.proposals).toHaveLength(0);
  });
});

describe('Undo It scenario', () => {
  it('blocks restore while running, stops, confirms, completes', () => {
    const blocked = play(undoItScenario, ['compare-lkg', 'restore-manual', 'try-restore-now']);
    expect(lastFeedback(blocked.feedback)?.tone).toBe('blocked');
    expect(isLessonComplete(undoItScenario, blocked.state)).toBe(false);

    const { state } = play(undoItScenario, [
      'compare-lkg',
      'restore-manual',
      'try-restore-now',
      'stop-process',
      'confirm-restore',
    ]);
    expect(isLessonComplete(undoItScenario, state)).toBe(true);
    expect(state.scene.undoPointCreated).toBe(true);
  });

  it('cancel-restore leaves the scene unchanged and incomplete', () => {
    const cancelled = play(undoItScenario, [
      'compare-manual',
      'restore-manual',
      'stop-process',
      'cancel-restore',
    ]);
    expect(lastFeedback(cancelled.feedback)?.tone).toBe('info');
    expect(isLessonComplete(undoItScenario, cancelled.state)).toBe(false);
    expect(cancelled.state.scene.restored).toBe(false);
  });

  it('maps snapshot preview intents to compare decisions', () => {
    const scene = undoItScenario.initialScene(0);
    const mapped = undoItScenario.intentToDecision?.(scene, {
      kind: 'preview-snapshot',
      snapshotId: 'lab:undo:snap:manual',
    });
    expect(mapped).toEqual({ kind: 'decision', decisionId: 'compare-manual' });
    const restore = undoItScenario.intentToDecision?.(scene, {
      kind: 'request-snapshot-restore',
      snapshotId: 'lab:undo:snap:manual',
    });
    expect(restore).toEqual({ kind: 'decision', decisionId: 'try-restore-now' });
  });

  it('restore makes the pre- and post-restore state visibly different', () => {
    const { state } = play(undoItScenario, [
      'compare-lkg',
      'restore-manual',
      'try-restore-now',
      'stop-process',
      'confirm-restore',
    ]);
    expect(state.scene.currentLabel).toContain('restored to');
    expect(state.scene.currentLabel).not.toContain('made things worse');
    expect(state.scene.restoredSummary).not.toBeNull();
    expect(state.scene.snapshots.some((snapshot) => snapshot.role === 'undo-restore')).toBe(true);
    expect(state.scene.snapshots.some((snapshot) => snapshot.id === 'lab:undo:snap:undo')).toBe(true);
  });
});
