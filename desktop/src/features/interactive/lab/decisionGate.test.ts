import { describe, expect, it } from 'vitest';
import { revalidateDecisionForConfirm, resolveDecisionGate, type DecisionConsequence } from './decisionGate';
import { initialLessonState } from './lessonEngine';
import { buildItScenario } from './scenarios/buildIt';
import { modItScenario } from './scenarios/modIt';

/** Drive Mod It to checkpoint 2 (conflict visible, unresolved). */
function modItCheckpoint2() {
  let state = initialLessonState(modItScenario);
  state = modItScenario.reduce(state, { kind: 'decision', decisionId: 'stage-better-caves' }).state;
  state = modItScenario.reduce(state, { kind: 'decision', decisionId: 'add-core-lib' }).state;
  state = modItScenario.reduce(state, { kind: 'decision', decisionId: 'include-nice-textures' }).state;
  return state;
}

/** The consequence a player would have reviewed when the dialog opened. */
function reviewedConsequence(): DecisionConsequence {
  const gate = resolveDecisionGate(modItScenario, modItCheckpoint2(), 'replace-terrain-overhaul');
  if (gate.status !== 'confirm') throw new Error('expected a confirm gate');
  return {
    danger: gate.decision.danger === true,
    confirmTitle: gate.decision.confirmTitle,
    confirmBody: gate.decision.confirmBody,
  };
}

describe('decisionGate (SOL-1 BLOCKER 2)', () => {
  it('dispatches a valid non-danger decision', () => {
    const state = initialLessonState(buildItScenario); // checkpoint 0
    const result = resolveDecisionGate(buildItScenario, state, 'place-version');
    expect(result.status).toBe('dispatch');
  });

  it('opens confirmation for a dangerous decision', () => {
    // Reach Mod It checkpoint 2 (conflict visible, not yet resolved).
    let state = initialLessonState(modItScenario);
    state = modItScenario.reduce(state, { kind: 'decision', decisionId: 'stage-better-caves' }).state;
    state = modItScenario.reduce(state, { kind: 'decision', decisionId: 'add-core-lib' }).state;
    state = modItScenario.reduce(state, { kind: 'decision', decisionId: 'include-nice-textures' }).state;
    expect(state.checkpoint).toBe(2);

    const result = resolveDecisionGate(modItScenario, state, 'replace-terrain-overhaul');
    expect(result.status).toBe('confirm');
    if (result.status === 'confirm') {
      expect(result.decision.id).toBe('replace-terrain-overhaul');
      expect(result.decision.confirmTitle).toBe('Confirm replacement');
    }
  });

  it('rejects a decision id that is not valid at the current checkpoint', () => {
    const state = initialLessonState(modItScenario); // checkpoint 0
    const result = resolveDecisionGate(modItScenario, state, 'apply-plan');
    expect(result.status).toBe('rejected');
    if (result.status === 'rejected') {
      expect(result.reason).toMatch(/not available/);
    }
  });

  it('rejects a disabled decision', () => {
    // Build It with Vanilla: the mod tile cannot be placed until switching to Fabric.
    let state = initialLessonState(buildItScenario);
    state = buildItScenario.reduce(state, { kind: 'decision', decisionId: 'place-version' }).state;
    state = buildItScenario.reduce(state, { kind: 'decision', decisionId: 'choose-vanilla' }).state;
    state = buildItScenario.reduce(state, { kind: 'decision', decisionId: 'name-it' }).state;
    // checkpoint 2 with Vanilla loader
    const result = resolveDecisionGate(buildItScenario, state, 'place-mod');
    expect(result.status).toBe('rejected');
    if (result.status === 'rejected') {
      expect(result.reason).toMatch(/Fabric/);
    }
  });
});

describe('revalidateDecisionForConfirm (SOL-1 BLOCKER 3 / residual BLOCKER B)', () => {
  it('accepts a still-valid dangerous decision whose consequence is unchanged', () => {
    const state = modItCheckpoint2();
    const result = revalidateDecisionForConfirm(modItScenario, state, 'replace-terrain-overhaul', reviewedConsequence());
    expect(result).toEqual({ valid: true });
  });

  it('rejects a confirmed decision that is no longer valid at the current checkpoint', () => {
    // Open confirmation at checkpoint 2, then the underlying state advances to
    // checkpoint 3 (conflict resolved). The old confirmation must not dispatch.
    let state = modItCheckpoint2();
    expect(revalidateDecisionForConfirm(modItScenario, state, 'replace-terrain-overhaul', reviewedConsequence()).valid).toBe(true);

    state = modItScenario.reduce(state, { kind: 'decision', decisionId: 'replace-terrain-overhaul' }).state;
    expect(state.checkpoint).toBe(3);

    const result = revalidateDecisionForConfirm(modItScenario, state, 'replace-terrain-overhaul', reviewedConsequence());
    expect(result).toEqual({ valid: false, reason: 'That action is no longer available at this step.' });
  });

  it('rejects a confirmed decision that became disabled', () => {
    let state = initialLessonState(buildItScenario);
    state = buildItScenario.reduce(state, { kind: 'decision', decisionId: 'place-version' }).state;
    state = buildItScenario.reduce(state, { kind: 'decision', decisionId: 'choose-vanilla' }).state;
    state = buildItScenario.reduce(state, { kind: 'decision', decisionId: 'name-it' }).state;
    const reviewed = { danger: false };
    const result = revalidateDecisionForConfirm(buildItScenario, state, 'place-mod', reviewed);
    expect(result).toEqual({ valid: false, reason: expect.stringMatching(/Fabric/) });
  });

  it('rejects the SAME decision id if its consequence copy changed after review', () => {
    const state = modItCheckpoint2();
    const reviewed = reviewedConsequence();
    expect(reviewed.confirmTitle).toBe('Confirm replacement');

    const changed = { ...reviewed, confirmTitle: 'Confirm something else' };
    const result = revalidateDecisionForConfirm(modItScenario, state, 'replace-terrain-overhaul', changed);
    expect(result).toEqual({ valid: false, reason: 'The consequences of this action changed. Please review it again.' });
  });

  it('rejects the same decision id if its confirm body changed after review', () => {
    const state = modItCheckpoint2();
    const reviewed = reviewedConsequence();
    const changed = { ...reviewed, confirmBody: reviewed.confirmBody + ' (amended)' };
    const result = revalidateDecisionForConfirm(modItScenario, state, 'replace-terrain-overhaul', changed);
    expect(result).toEqual({ valid: false, reason: expect.stringMatching(/consequences of this action changed/) });
  });

  it('rejects the same decision id if it became non-dangerous after review', () => {
    const state = modItCheckpoint2();
    const reviewed = { ...reviewedConsequence(), danger: false };
    const result = revalidateDecisionForConfirm(modItScenario, state, 'replace-terrain-overhaul', reviewed);
    expect(result).toEqual({ valid: false, reason: expect.stringMatching(/consequences of this action changed/) });
  });
});
