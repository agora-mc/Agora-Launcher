import { describe, expect, it } from 'vitest';
import { initialLessonState, isLessonComplete, reduceLesson } from './lessonEngine';
import type { FeedbackEvent, LabLessonState, LabScenario } from './scenarioTypes';
import { healItScenario } from './scenarios/healIt';
import { fixItScenario } from './scenarios/fixIt';
import { takeItOfflineScenario } from './scenarios/takeItOffline';

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

function lastTone(feedback: FeedbackEvent[]): FeedbackEvent['tone'] | undefined {
  return feedback[feedback.length - 1]?.tone;
}

describe('Heal It scenario', () => {
  it('completes: scan, fix loader, resolve warning, keep auto memory, revalidate', () => {
    const { state, feedback } = play(healItScenario, [
      'run-validation',
      'choose-fabric',
      'use-agora-java',
      'keep-memory-auto',
      'revalidate',
    ]);
    expect(isLessonComplete(healItScenario, state)).toBe(true);
    expect(state.scene.findings).toHaveLength(0);
    expect(lastTone(feedback)).toBe('success');
  });

  it('a wrong loader is blocked and the player can recover', () => {
    const blocked = play(healItScenario, ['run-validation', 'choose-forge']);
    expect(lastTone(blocked.feedback)).toBe('blocked');
    expect(isLessonComplete(healItScenario, blocked.state)).toBe(false);
    expect(blocked.state.scene.blockerCleared).toBe(false);

    const { state } = play(healItScenario, [
      'run-validation',
      'choose-forge',
      'choose-fabric',
      'use-agora-java',
      'keep-memory-auto',
      'revalidate',
    ]);
    expect(isLessonComplete(healItScenario, state)).toBe(true);
  });

  it('indeterminate loader does not clear the blocker', () => {
    const quilt = play(healItScenario, ['run-validation', 'choose-quilt']);
    expect(lastTone(quilt.feedback)).toBe('caution');
    expect(quilt.state.scene.blockerCleared).toBe(false);
  });

  it('staging manual memory is a peer choice that still completes', () => {
    const { state } = play(healItScenario, [
      'run-validation',
      'choose-fabric',
      'use-agora-java',
      'stage-memory-manual',
      'revalidate',
    ]);
    expect(isLessonComplete(healItScenario, state)).toBe(true);
    expect(state.scene.runtime.memory.proposedMiB).toBe(4096);
  });

  it('maps loader and memory intents to proposed decisions', () => {
    const scene = healItScenario.initialScene(1);
    const fabric = healItScenario.intentToDecision?.(scene, { kind: 'review-loader', candidateId: 'lab:heal:loader:fabric' });
    expect(fabric).toEqual({ decisionId: 'choose-fabric' });
    const manual = healItScenario.intentToDecision?.(scene, { kind: 'propose-memory', mode: 'manual', memoryMiB: 4096 });
    expect(manual).toEqual({ decisionId: 'stage-memory-manual' });
  });
});

describe('Fix It scenario', () => {
  it('completes: read evidence, pick the mod hypothesis, run experiment, confirm', () => {
    const { state, feedback } = play(fixItScenario, [
      'review-evidence',
      'hypothesis-mod',
      'run-experiment',
      'confirm-success',
    ]);
    expect(isLessonComplete(fixItScenario, state)).toBe(true);
    expect(state.scene.outcome).toBe('recovered');
    expect(state.scene.recoveryPointCreated).toBe(true);
    expect(lastTone(feedback)).toBe('success');
  });

  it('a wrong hypothesis is less likely and the player can test the mod instead', () => {
    const wrong = play(fixItScenario, ['review-evidence', 'hypothesis-memory', 'run-experiment']);
    expect(wrong.state.scene.outcome).toBe('unchanged');
    expect(lastTone(wrong.feedback)).toBe('caution');
    expect(isLessonComplete(fixItScenario, wrong.state)).toBe(false);

    const { state } = play(fixItScenario, [
      'review-evidence',
      'hypothesis-memory',
      'run-experiment',
      'test-mod-instead',
      'confirm-success',
    ]);
    expect(isLessonComplete(fixItScenario, state)).toBe(true);
    expect(state.scene.outcome).toBe('recovered');
  });

  it('cancelling an experiment restores the instance and completes safely', () => {
    const cancelled = play(fixItScenario, [
      'review-evidence',
      'hypothesis-world',
      'run-experiment',
      'restore-instance',
    ]);
    expect(isLessonComplete(fixItScenario, cancelled.state)).toBe(true);
    expect(cancelled.state.scene.cancelled).toBe(true);
    expect(cancelled.state.scene.playerConfirmed).toBe(false);
    expect(lastTone(cancelled.feedback)).toBe('info');
  });

  it('a successful experiment still requires player confirmation', () => {
    const ran = play(fixItScenario, ['review-evidence', 'hypothesis-mod', 'run-experiment']);
    expect(isLessonComplete(fixItScenario, ran.state)).toBe(false);
    expect(ran.state.scene.outcome).toBe('recovered');
  });
});

describe('Take It Offline scenario', () => {
  it('completes: inspect, download, delegated, recheck', () => {
    const { state, feedback } = play(takeItOfflineScenario, [
      'inspect-delegated',
      'download-content',
      'use-delegated',
      'recheck',
    ]);
    expect(isLessonComplete(takeItOfflineScenario, state)).toBe(true);
    expect(state.scene.readiness.overall).toBe('ready');
    expect(lastTone(feedback)).toBe('success');
  });

  it('leaving content missing makes the final recheck needs-attention', () => {
    const { state, feedback } = play(takeItOfflineScenario, [
      'inspect-delegated',
      'leave-missing',
      'use-delegated',
      'recheck',
    ]);
    expect(isLessonComplete(takeItOfflineScenario, state)).toBe(true);
    expect(state.scene.readiness.overall).toBe('needs-attention');
    expect(lastTone(feedback)).toBe('caution');
  });

  it('direct launch is blocked by policy for sign-in', () => {
    const direct = play(takeItOfflineScenario, [
      'inspect-delegated',
      'download-content',
      'use-direct',
    ]);
    expect(lastTone(direct.feedback)).toBe('caution');
    const signIn = direct.state.scene.readiness.checks.find((check) => check.category === 'sign-in-and-launch');
    expect(signIn?.state).toBe('blocked-by-policy');
  });

  it('maps re-check intent to the recheck decision', () => {
    const scene = takeItOfflineScenario.initialScene(3);
    const mapped = takeItOfflineScenario.intentToDecision?.(scene, { kind: 'review-offline-readiness' });
    expect(mapped).toEqual({ decisionId: 'recheck' });
  });
});
