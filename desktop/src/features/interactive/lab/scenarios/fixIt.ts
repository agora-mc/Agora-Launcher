/**
 * Fix It scenario: Crash Doctor as a hypothesis-driven, recoverable experiment.
 *
 * Sol-0 contract: `docs/interactive/LESSON_MAP.md` §5. Suspects are hypotheses,
 * not verdicts. A recovery point comes before the first mutation, and one
 * change is tested at a time. A successful launch supports a hypothesis but
 * does not prove a cause. Everything is authored, deterministic, and
 * namespaced `lab:fix:*`.
 */

import type { VisualCrashEvidence, VisualId, VisualScene } from '../../domain/models';
import type {
  LabCheckpoint,
  LabDecision,
  LabLessonState,
  LabReduction,
  LabScenario,
} from '../scenarioTypes';
import { simId } from '../scenarioTypes';

const NS = 'fix';

const IDS = {
  hypothesisMod: simId(NS, 'hyp', 'mod'),
  hypothesisMemory: simId(NS, 'hyp', 'memory'),
  hypothesisWorld: simId(NS, 'hyp', 'world'),
};

export type FixOutcome = 'recovered' | 'unchanged' | 'changed';

/**
 * The concrete change each experiment makes. Every hypothesis is tested by a
 * DIFFERENT single change; describing them all as "disable one mod" taught the
 * wrong diagnostic method for the memory and world hypotheses (T6-5).
 */
function experimentChange(hypothesisId: string | null): string {
  if (hypothesisId === IDS.hypothesisMemory) return 'raises the memory allocation by one step';
  if (hypothesisId === IDS.hypothesisWorld) return 'opens a new temporary world instead of the usual one';
  return 'disables the startup mod named in the crash report';
}

export interface FixItScene extends VisualScene {
  evidence: VisualCrashEvidence;
  selectedHypothesisId: VisualId | null;
  experimentRan: boolean;
  outcome: FixOutcome | null;
  recoveryPointCreated: boolean;
  cancelled: boolean;
  playerConfirmed: boolean;
}

function baseEvidence(): VisualCrashEvidence {
  return {
    incidentLabel: 'Crash on launch — Aug 9',
    evidenceSources: [
      { kind: 'crash-report', state: 'known', summary: 'Names a mod class during startup.' },
      { kind: 'log', state: 'known', summary: 'Shows an out-of-memory message near the end.' },
      { kind: 'process-outcome', state: 'known', summary: 'Game closed within seconds of launching.' },
      { kind: 'health', state: 'known', summary: 'Health scan was green before launch.' },
    ],
    hypotheses: [
      {
        id: IDS.hypothesisMod,
        title: 'A mod fails during startup',
        strength: 'high',
        supportingClues: ['Crash report names a mod class.'],
        contradictoryClues: [],
        state: 'candidate',
      },
      {
        id: IDS.hypothesisMemory,
        title: 'Not enough memory for this world',
        strength: 'medium',
        supportingClues: ['Log shows an out-of-memory message.'],
        contradictoryClues: ['The world is small.'],
        state: 'candidate',
      },
      {
        id: IDS.hypothesisWorld,
        title: 'A corrupt world file',
        strength: 'low',
        supportingClues: ['Crash mentions a world file.'],
        contradictoryClues: ['This is a fresh world.'],
        state: 'candidate',
      },
    ],
    experiment: { phase: 'read-only', recoveryReady: true },
    privacyNote: 'Evidence stays on this device. Full logs are not shared by default.',
  };
}

function withHypothesisState(
  evidence: VisualCrashEvidence,
  hypothesisId: VisualId,
  state: 'candidate' | 'testing' | 'less-likely' | 'inconclusive',
): VisualCrashEvidence {
  return {
    ...evidence,
    hypotheses: evidence.hypotheses.map((hypothesis) =>
      hypothesis.id === hypothesisId ? { ...hypothesis, state } : hypothesis,
    ),
  };
}

function canonicalScene(checkpoint: number): FixItScene {
  const scene: FixItScene = {
    source: { kind: 'simulation', scenarioId: NS, scenarioVersion: 1 },
    content: [],
    relationships: [],
    findings: [],
    proposals: [],
    evidence: baseEvidence(),
    selectedHypothesisId: null,
    experimentRan: false,
    outcome: null,
    recoveryPointCreated: false,
    cancelled: false,
    playerConfirmed: false,
  };
  // Checkpoint 1 is "clues read, nothing chosen yet". Pre-selecting a
  // hypothesis here asserted a choice the player had not made (T6-2 class).
  if (checkpoint >= 1) {
    scene.evidence = { ...scene.evidence, experiment: { ...scene.evidence.experiment, phase: 'read-only' } };
  }
  if (checkpoint >= 2) {
    scene.experimentRan = true;
    scene.recoveryPointCreated = true;
    scene.outcome = 'recovered';
    scene.evidence = {
      ...withHypothesisState(scene.evidence, IDS.hypothesisMod, 'testing'),
      experiment: { phase: 'awaiting-player-confirmation', summary: 'Disabled one mod and launched.', recoveryReady: true },
    };
  }
  return scene;
}

function decisionsFor(state: LabLessonState<FixItScene>): LabDecision[] {
  const scene = state.scene;
  if (state.checkpoint === 0) {
    return [
      { id: 'review-evidence', label: 'Read the clues' },
    ];
  }
  if (state.checkpoint === 1) {
    return [
      { id: 'hypothesis-mod', label: 'Test: a mod fails during startup' },
      { id: 'hypothesis-memory', label: 'Test: not enough memory' },
      { id: 'hypothesis-world', label: 'Test: a corrupt world file' },
    ];
  }
  if (state.checkpoint === 2) {
    const decisions: LabDecision[] = [];
    if (!scene.experimentRan) {
      const selected = scene.evidence.hypotheses.find((h) => h.id === scene.selectedHypothesisId);
      decisions.push({
        id: 'run-experiment',
        label: 'Create recovery point, then test one change',
        danger: true,
        confirmTitle: 'Confirm experiment',
        // Names the hypothesis AND the change this specific experiment makes.
        // A static body claimed "one mod is disabled" even when the player was
        // testing memory, which taught the wrong diagnostic method (T6-5).
        confirmBody: `A simulated recovery point is created first. To test "${
          selected?.title ?? 'the selected hypothesis'
        }", this ${experimentChange(scene.selectedHypothesisId)} and launches the game once. Nothing else is changed.`,
      });
    }
    if (scene.outcome === 'recovered' && !scene.playerConfirmed) {
      decisions.push({ id: 'confirm-success', label: 'Confirm: crash is gone' });
    }
    if (scene.outcome && scene.outcome !== 'recovered') {
      decisions.push({
        id: 'test-mod-instead',
        label: 'Test the startup mod instead',
        danger: true,
        confirmTitle: 'Confirm new experiment',
        confirmBody: 'A recovery point exists. This disables the startup mod and launches once — one change at a time.',
      });
      decisions.push({
        id: 'restore-instance',
        label: 'Stop and restore from the recovery point',
        danger: true,
        confirmTitle: 'Confirm restore',
        confirmBody: 'This returns the simulated instance to the recovery point and keeps it unchanged.',
      });
    }
    return decisions;
  }
  return [];
}

const checkpoints: LabCheckpoint<FixItScene>[] = [
  {
    id: 'evidence',
    goal: 'Read the clues before guessing what caused the crash.',
    expectedModel: 'Crash Doctor builds hypotheses from local evidence, not from a verdict.',
    decisionsFor,
  },
  {
    id: 'hypothesis',
    goal: 'Pick a likely explanation to test. Strength is not certainty.',
    expectedModel: 'A hypothesis is something to test, not a proven cause.',
    decisionsFor,
  },
  {
    id: 'experiment',
    goal: 'A recovery point comes first; then change one variable at a time.',
    expectedModel: 'Experiments are bounded and recoverable; one successful launch does not prove a cause.',
    decisionsFor,
  },
];

function reduce(
  state: LabLessonState<FixItScene>,
  event: { kind: 'decision'; decisionId: string },
): LabReduction<FixItScene> {
  const scene = state.scene;
  const { decisionId } = event;

  if (decisionId === 'review-evidence') {
    return {
      state: { ...state, checkpoint: 1, lastFeedback: null },
      feedback: {
        tone: 'info',
        message: 'Three clues: a mod class in the report, an out-of-memory line, and a green health scan.',
      },
    };
  }

  if (['hypothesis-mod', 'hypothesis-memory', 'hypothesis-world'].includes(decisionId)) {
    const targetId =
      decisionId === 'hypothesis-mod' ? IDS.hypothesisMod : decisionId === 'hypothesis-memory' ? IDS.hypothesisMemory : IDS.hypothesisWorld;
    const next = { ...scene, selectedHypothesisId: targetId };
    const target = next.evidence.hypotheses.find((h) => h.id === targetId);
    // Mark the hypothesis actually under test. It was previously set back to
    // `candidate`, so at the moment the player committed to a serious
    // experiment nothing on screen showed WHAT was being tested (T6-5).
    next.evidence = {
      ...withHypothesisState(next.evidence, targetId, 'testing'),
      experiment: {
        phase: 'proposed',
        summary: `Testing: ${target?.title ?? 'the selected hypothesis'} — ${experimentChange(targetId)}, then launch once.`,
        recoveryReady: true,
      },
    };
    return {
      state: { ...state, checkpoint: 2, scene: next, lastFeedback: null },
      feedback: {
        tone: 'info',
        message: `Testing "${target?.title ?? 'hypothesis'}" (strength: ${target?.strength}). The experiment will ${experimentChange(targetId)} and launch once.`,
      },
    };
  }

  if (decisionId === 'run-experiment' && !scene.experimentRan) {
    const hypothesis = scene.evidence.hypotheses.find((h) => h.id === scene.selectedHypothesisId);
    const outcome: FixOutcome = hypothesis?.id === IDS.hypothesisMod ? 'recovered' : hypothesis?.id === IDS.hypothesisMemory ? 'unchanged' : 'changed';
    const next: FixItScene = {
      ...scene,
      experimentRan: true,
      recoveryPointCreated: true,
      outcome,
    };
    const change = experimentChange(scene.selectedHypothesisId);
    const summary =
      outcome === 'recovered'
        ? `Disabled the startup mod, launched once — the crash did not happen.`
        : outcome === 'unchanged'
          ? `Tried "${hypothesis?.title ?? 'the hypothesis'}": ${change}, launched once — the crash happened again, unchanged.`
          : `Tried "${hypothesis?.title ?? 'the hypothesis'}": ${change}, launched once — a different crash appeared.`;
    next.evidence = {
      ...withHypothesisState(
        next.evidence,
        scene.selectedHypothesisId ?? '',
        outcome === 'recovered' ? 'testing' : 'less-likely',
      ),
      experiment: {
        // Only a supporting result awaits the player's confirmation. A
        // disproved experiment is finished — leaving "Awaiting your
        // confirmation" beside its own result contradicted the screen (T6-10).
        phase: outcome === 'recovered' ? 'awaiting-player-confirmation' : 'complete',
        summary,
        recoveryReady: true,
      },
    };
    return {
      state: { ...state, checkpoint: 2, scene: next, lastFeedback: null },
      feedback: {
        tone: outcome === 'recovered' ? 'success' : 'caution',
        message: outcome === 'recovered'
          ? 'The crash did not happen this time. One launch supports the hypothesis — it does not prove the cause.'
          // SOL-3 P2 #1: teach hypothesis REVISION, not just "pick the other
          // one" — say why this clue was plausible and what it now points to.
          : outcome === 'unchanged'
            ? 'The crash happened again, so more memory was not the cause. The out-of-memory line was real, but it appeared while a mod was still loading — a symptom, not the cause. The crash report names that mod, so it is now the stronger explanation.'
            : 'The crash changed, so the world file was not the original cause — a different fault surfaced instead. This is a new observation, not success. The crash report still names a mod during startup.',
      },
    };
  }

  if (decisionId === 'confirm-success' && scene.outcome === 'recovered' && !scene.playerConfirmed) {
    const next: FixItScene = { ...scene, playerConfirmed: true };
    next.evidence = {
      ...next.evidence,
      experiment: { phase: 'complete', summary: 'You confirmed the crash did not return after the change.', recoveryReady: true },
    };
    return {
      state: { ...state, status: 'complete', scene: next, lastFeedback: null },
      feedback: {
        tone: 'success',
        message: 'Recovery confirmed. Remember: one successful launch makes the hypothesis more likely — it does not prove the cause by itself.',
      },
    };
  }

  if (decisionId === 'test-mod-instead' && scene.outcome && scene.outcome !== 'recovered') {
    const next: FixItScene = {
      ...scene,
      selectedHypothesisId: IDS.hypothesisMod,
      outcome: 'recovered',
      experimentRan: true,
      recoveryPointCreated: true,
    };
    next.evidence = {
      ...withHypothesisState(next.evidence, IDS.hypothesisMod, 'testing'),
      experiment: {
        phase: 'awaiting-player-confirmation',
        summary: 'Disabled the startup mod, launched once — the crash did not happen.',
        recoveryReady: true,
      },
    };
    return {
      state: { ...state, scene: next, lastFeedback: null },
      feedback: {
        tone: 'success',
        message: 'The startup-mod test did not crash. Confirm the recovery to finish the practice.',
      },
    };
  }

  if (decisionId === 'restore-instance' && scene.outcome && scene.outcome !== 'recovered') {
    const next: FixItScene = { ...scene, cancelled: true };
    next.evidence = {
      ...next.evidence,
      experiment: { phase: 'complete', summary: 'Experiment cancelled — the simulated instance was restored.', recoveryReady: true },
    };
    return {
      state: { ...state, status: 'complete', scene: next, lastFeedback: null },
      feedback: {
        tone: 'info',
        message: 'Experiment cancelled. The simulated instance was restored from the recovery point, unchanged.',
      },
    };
  }

  return { state, feedback: null };
}

export const fixItScenario: LabScenario<FixItScene> = {
  id: NS,
  version: 1,
  title: 'Fix It',
  shortTitle: 'Fix It',
  description: 'Test one likely cause with a recovery point, then confirm.',
  iconLabel: '🔍',
  checkpoints,
  guideTopics: ['crash-recovery'],
  realDestinations: [
    { type: 'tab', tab: 'instances' },
  ],
  initialScene: canonicalScene,
  // Checkpoint 2+ exists only after a specific hypothesis was tested and an
  // outcome observed; neither is recorded in progress. Resume to checkpoint 1
  // and let the player choose again rather than inheriting someone else's
  // experiment (SOL §22 / T6-2).
  safeResumeCheckpoint: (checkpoint: number) => Math.min(checkpoint, 1),
  reduce,
  successPredicate(state) {
    return state.status === 'complete';
  },
  completionMessage: 'You completed the practice: a hypothesis was tested with one change, recovery stayed ready, and one launch was treated as supporting — not proving — a cause.',
};
